use std::{collections::HashMap, fs, path::Path, sync::Arc};

use jiff::tz::TimeZone as JiffTimeZone;
use serde::Deserialize;

use crate::{
    LoadedEntry, PricingMap, Result, TimestampMs, TokenUsageRaw, UsageEntry, UsageMessage,
    cli::CostMode, fast::LinePrefilter, format_date_tz, missing_pricing_model_for_candidates,
};
use turbotokens_adapter_common::jsonl;

const LONG_CONTEXT_THRESHOLD: u64 = 200_000;
const UNKNOWN_MODEL: &str = "unknown";

#[derive(Debug, Deserialize)]
struct GrokTurnLine {
    #[serde(default, deserialize_with = "jsonl::non_empty_string")]
    r#type: Option<String>,
    #[serde(default, deserialize_with = "jsonl::non_empty_string")]
    ts: Option<String>,
    #[serde(default, deserialize_with = "jsonl::non_empty_string")]
    session_id: Option<String>,
    #[serde(default, deserialize_with = "jsonl::non_empty_string")]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GrokLogLine {
    #[serde(default, deserialize_with = "jsonl::non_empty_string")]
    ts: Option<String>,
    #[serde(default, deserialize_with = "jsonl::non_empty_string")]
    sid: Option<String>,
    #[serde(default, deserialize_with = "jsonl::non_empty_string")]
    msg: Option<String>,
    ctx: Option<GrokUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct GrokUsage {
    #[serde(default, deserialize_with = "jsonl::lenient_u64")]
    prompt_tokens: u64,
    #[serde(default, deserialize_with = "jsonl::lenient_u64")]
    cached_prompt_tokens: u64,
    #[serde(default, deserialize_with = "jsonl::lenient_u64")]
    completion_tokens: u64,
    #[serde(default, deserialize_with = "jsonl::lenient_u64")]
    reasoning_tokens: u64,
    #[serde(default, deserialize_with = "jsonl::lenient_u64")]
    loop_index: u64,
}

#[derive(Debug, Clone)]
pub(super) struct ModelEvent {
    pub(super) timestamp: TimestampMs,
    pub(super) model: String,
}

pub(super) type ModelTimelines = HashMap<String, Vec<ModelEvent>>;

#[derive(Debug, Clone)]
pub(super) struct GrokUsageEntry {
    timestamp: TimestampMs,
    timestamp_text: String,
    session_id: String,
    model: String,
    message_id: String,
    prompt_tokens: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    reasoning_tokens: u64,
}

pub(super) fn read_model_events(path: &Path) -> Result<Vec<(String, ModelEvent)>> {
    let content = fs::read(path)?;
    let prefilter = LinePrefilter::all(&[br#""turn_started""#]);
    Ok(jsonl::records::<GrokTurnLine>(&content, Some(&prefilter))
        .filter_map(turn_line_to_event)
        .collect())
}

fn turn_line_to_event(line: GrokTurnLine) -> Option<(String, ModelEvent)> {
    if line.r#type.as_deref() != Some("turn_started") {
        return None;
    }
    let timestamp = crate::parse_ts_timestamp(line.ts.as_deref()?)?;
    Some((
        line.session_id?,
        ModelEvent {
            timestamp,
            model: line.model_id?,
        },
    ))
}

pub(super) fn read_usage_log(
    path: &Path,
    timelines: &ModelTimelines,
) -> Result<Vec<GrokUsageEntry>> {
    let content = fs::read(path)?;
    let prefilter = LinePrefilter::all(&[br#""shell.turn.inference_done""#]);
    Ok(jsonl::records::<GrokLogLine>(&content, Some(&prefilter))
        .filter_map(|line| log_line_to_entry(line, timelines))
        .collect())
}

pub(super) fn usage_line_from_bytes(
    line: &[u8],
    timelines: &ModelTimelines,
) -> Option<GrokUsageEntry> {
    let text = std::str::from_utf8(line).ok()?;
    if !text.contains("shell.turn.inference_done") {
        return None;
    }
    log_line_to_entry(serde_json::from_str(text).ok()?, timelines)
}

pub(super) fn turn_event_from_bytes(line: &[u8]) -> Option<(String, ModelEvent)> {
    let text = std::str::from_utf8(line).ok()?;
    if !text.contains("turn_started") {
        return None;
    }
    turn_line_to_event(serde_json::from_str(text).ok()?)
}

pub(super) fn record_model_event(
    timelines: &mut ModelTimelines,
    session_id: String,
    event: ModelEvent,
) {
    let events = timelines.entry(session_id).or_default();
    if events
        .iter()
        .any(|existing| existing.timestamp == event.timestamp && existing.model == event.model)
    {
        return;
    }
    let index = events.partition_point(|existing| existing.timestamp <= event.timestamp);
    events.insert(index, event);
}

fn log_line_to_entry(line: GrokLogLine, timelines: &ModelTimelines) -> Option<GrokUsageEntry> {
    if line.msg.as_deref() != Some("shell.turn.inference_done") {
        return None;
    }
    let timestamp_text = line.ts?;
    let timestamp = crate::parse_ts_timestamp(&timestamp_text)?;
    let session_id = line.sid?;
    let usage = line.ctx?;
    let cache_read_tokens = usage.cached_prompt_tokens.min(usage.prompt_tokens);
    let input_tokens = usage.prompt_tokens.saturating_sub(cache_read_tokens);
    if usage.prompt_tokens == 0 && usage.completion_tokens == 0 && usage.reasoning_tokens == 0 {
        return None;
    }
    let model = model_at(timelines.get(&session_id), timestamp)
        .unwrap_or(UNKNOWN_MODEL)
        .to_string();
    Some(GrokUsageEntry {
        message_id: format!(
            "grok:{session_id}:{}:{}",
            timestamp.as_millis(),
            usage.loop_index
        ),
        timestamp,
        timestamp_text,
        session_id,
        model,
        prompt_tokens: usage.prompt_tokens,
        input_tokens,
        output_tokens: usage.completion_tokens,
        cache_read_tokens,
        reasoning_tokens: usage.reasoning_tokens,
    })
}

fn model_at(events: Option<&Vec<ModelEvent>>, timestamp: TimestampMs) -> Option<&str> {
    let events = events?;
    let index = events.partition_point(|event| event.timestamp <= timestamp);
    index
        .checked_sub(1)
        .and_then(|index| events.get(index))
        .or_else(|| events.first())
        .map(|event| event.model.as_str())
}

pub(super) fn grok_entry_key(entry: &GrokUsageEntry) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        entry.session_id,
        entry.timestamp_text,
        entry.input_tokens,
        entry.output_tokens,
        entry.cache_read_tokens,
        entry.reasoning_tokens,
        entry.model
    )
}

pub(super) fn grok_entry_to_loaded(
    entry: GrokUsageEntry,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: &PricingMap,
) -> LoadedEntry {
    let usage = TokenUsageRaw {
        input_tokens: entry.input_tokens,
        output_tokens: entry.output_tokens,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: entry.cache_read_tokens,
        speed: None,
        cache_creation: None,
    };
    let cost = calculate_grok_cost(&entry, mode, pricing);
    let missing_pricing_model = missing_grok_pricing(&entry, mode, pricing);
    let data = UsageEntry {
        session_id: Some(entry.session_id.clone()),
        timestamp: entry.timestamp_text.clone(),
        version: None,
        message: UsageMessage {
            usage,
            model: Some(entry.model.clone()),
            id: Some(entry.message_id),
        },
        cost_usd: None,
        request_id: None,
        is_api_error_message: None,
        is_sidechain: None,
    };
    LoadedEntry {
        date: format_date_tz(entry.timestamp, tz),
        timestamp: entry.timestamp,
        project: Arc::from("grok"),
        session_id: Arc::from(entry.session_id),
        project_path: Arc::from("Grok Build"),
        cost,
        extra_total_tokens: entry.reasoning_tokens,
        credits: None,
        message_count: None,
        model: Some(entry.model),
        usage_limit_reset_time: None,
        missing_pricing_model,
        data,
    }
}

fn calculate_grok_cost(entry: &GrokUsageEntry, mode: CostMode, pricing: &PricingMap) -> f64 {
    if mode == CostMode::Display {
        return 0.0;
    }
    for candidate in model_candidates(&entry.model) {
        let Some(model_pricing) = pricing.find(&candidate) else {
            continue;
        };
        let long_context = entry.prompt_tokens >= LONG_CONTEXT_THRESHOLD;
        let rate = |base: f64, above: Option<f64>| {
            if long_context {
                above.unwrap_or(base)
            } else {
                base
            }
        };
        return entry.input_tokens as f64
            * rate(model_pricing.input, model_pricing.input_above_200k)
            + entry.cache_read_tokens as f64
                * rate(
                    model_pricing.cache_read,
                    model_pricing.cache_read_above_200k,
                )
            + entry.output_tokens.saturating_add(entry.reasoning_tokens) as f64
                * rate(model_pricing.output, model_pricing.output_above_200k);
    }
    0.0
}

fn missing_grok_pricing(
    entry: &GrokUsageEntry,
    mode: CostMode,
    pricing: &PricingMap,
) -> Option<String> {
    if mode == CostMode::Display {
        return None;
    }
    missing_pricing_model_for_candidates(
        &entry.model,
        model_candidates(&entry.model),
        entry
            .prompt_tokens
            .saturating_add(entry.output_tokens)
            .saturating_add(entry.reasoning_tokens),
        Some(pricing),
    )
}

fn model_candidates(model: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    match model {
        "grok-build" | "grok-build-latest" => candidates.push("grok-4.5".to_string()),
        value if value == "grok-4.20" || value.starts_with("grok-4.20-") => {
            candidates.push("grok-4.20".to_string());
        }
        "grok-code-fast" | "grok-code-fast-1" | "grok-code-fast-1-0825" => {
            candidates.push("grok-build-0.1".to_string());
        }
        _ => {}
    }
    candidates.push(format!("xai/{model}"));
    candidates.push(model.to_string());
    candidates.sort();
    candidates.dedup();
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(model: &str) -> ModelTimelines {
        HashMap::from([(
            "session-a".to_string(),
            vec![ModelEvent {
                timestamp: crate::parse_ts_timestamp("2026-07-08T00:00:00.000Z").unwrap(),
                model: model.to_string(),
            }],
        )])
    }

    fn entry(prompt_tokens: u64, cached_prompt_tokens: u64) -> GrokUsageEntry {
        log_line_to_entry(
            GrokLogLine {
                ts: Some("2026-07-08T07:10:13.766Z".to_string()),
                sid: Some("session-a".to_string()),
                msg: Some("shell.turn.inference_done".to_string()),
                ctx: Some(GrokUsage {
                    prompt_tokens,
                    cached_prompt_tokens,
                    completion_tokens: 251,
                    reasoning_tokens: 99,
                    loop_index: 3,
                }),
            },
            &timeline("grok-4.5"),
        )
        .unwrap()
    }

    #[test]
    fn parses_anonymized_real_grok_inference_shape_without_double_counting_cache() {
        let entry = entry(36_458, 32_876);
        let loaded = grok_entry_to_loaded(
            entry,
            None,
            CostMode::Calculate,
            &PricingMap::load_embedded(),
        );

        assert_eq!(loaded.data.message.usage.input_tokens, 3_582);
        assert_eq!(loaded.data.message.usage.cache_read_input_tokens, 32_876);
        assert_eq!(loaded.data.message.usage.output_tokens, 251);
        assert_eq!(loaded.extra_total_tokens, 99);
        assert_eq!(
            crate::total_usage_tokens(loaded.data.message.usage) + loaded.extra_total_tokens,
            36_808
        );
        assert_eq!(loaded.model.as_deref(), Some("grok-4.5"));
        assert!(loaded.cost > 0.0);
    }

    #[test]
    fn prices_the_whole_request_at_long_context_rates_from_the_prompt_total() {
        let pricing = PricingMap::load_embedded();
        let short = entry(199_999, 190_000);
        let long = entry(200_000, 190_000);

        let short_cost = calculate_grok_cost(&short, CostMode::Calculate, &pricing);
        let long_cost = calculate_grok_cost(&long, CostMode::Calculate, &pricing);

        assert!(long_cost > short_cost * 1.9);
    }

    #[test]
    fn prices_official_grok_420_version_slugs_as_the_grok_420_family() {
        let pricing = PricingMap::load_embedded();
        let entry = log_line_to_entry(
            GrokLogLine {
                ts: Some("2026-07-08T07:10:13.766Z".to_string()),
                sid: Some("session-a".to_string()),
                msg: Some("shell.turn.inference_done".to_string()),
                ctx: Some(GrokUsage {
                    prompt_tokens: 10,
                    cached_prompt_tokens: 4,
                    completion_tokens: 2,
                    reasoning_tokens: 1,
                    ..GrokUsage::default()
                }),
            },
            &timeline("grok-4.20-0309-reasoning"),
        )
        .unwrap();

        assert_eq!(model_candidates(&entry.model)[0], "grok-4.20");
        assert!(
            (calculate_grok_cost(&entry, CostMode::Calculate, &pricing) - 0.000_015_8).abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn keeps_usage_with_unknown_model_instead_of_losing_tokens() {
        let entry = log_line_to_entry(
            GrokLogLine {
                ts: Some("2026-07-08T07:10:13.766Z".to_string()),
                sid: Some("missing-session".to_string()),
                msg: Some("shell.turn.inference_done".to_string()),
                ctx: Some(GrokUsage {
                    prompt_tokens: 10,
                    completion_tokens: 2,
                    ..GrokUsage::default()
                }),
            },
            &ModelTimelines::new(),
        )
        .unwrap();

        assert_eq!(entry.model, UNKNOWN_MODEL);
        assert_eq!(entry.input_tokens, 10);
    }

    #[test]
    fn attributes_usage_to_the_model_active_at_inference_time() {
        let timelines = HashMap::from([(
            "session-a".to_string(),
            vec![
                ModelEvent {
                    timestamp: crate::parse_ts_timestamp("2026-07-08T00:00:00.000Z").unwrap(),
                    model: "grok-build-0.1".to_string(),
                },
                ModelEvent {
                    timestamp: crate::parse_ts_timestamp("2026-07-08T08:00:00.000Z").unwrap(),
                    model: "grok-4.5".to_string(),
                },
            ],
        )]);
        let parse_at = |timestamp: &str| {
            log_line_to_entry(
                GrokLogLine {
                    ts: Some(timestamp.to_string()),
                    sid: Some("session-a".to_string()),
                    msg: Some("shell.turn.inference_done".to_string()),
                    ctx: Some(GrokUsage {
                        prompt_tokens: 10,
                        completion_tokens: 2,
                        ..GrokUsage::default()
                    }),
                },
                &timelines,
            )
            .unwrap()
        };

        assert_eq!(parse_at("2026-07-08T07:59:59.999Z").model, "grok-build-0.1");
        assert_eq!(parse_at("2026-07-08T08:00:00.000Z").model, "grok-4.5");
    }
}
