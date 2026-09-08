use turbotokens_adapter_common::{chunk_file_indexes_by_size, read_files_parallel};
use turbotokens_core::*;

mod cache;
mod daily;
mod live;
mod paths;
mod resident;
mod watch;
#[cfg(windows)]
mod windows_change_time;

use std::{
    hash::{Hash, Hasher},
    path::Path,
    sync::Arc,
};

use jiff::tz::TimeZone as JiffTimeZone;
use memchr::{memchr, memmem};
use rustc_hash::FxHasher;
use serde::Deserialize;

use crate::{
    LoadedEntry, LoadedFile, PricingMap, Result, Speed, TimestampMs, TokenUsageRaw, UsageEntry,
    UsageSummary, calculate_cost, calculate_cost_for_usage,
    cli::{CostMode, SharedArgs},
    debug_log,
    fast::{FxHashMap, SmallIndexVec, suffix_string},
    format_date_tz, log_level, missing_pricing_model_for_usage, parse_ts_timestamp, parse_tz,
    progress,
};

pub use live::run_live;
#[doc(hidden)]
pub use paths::timestamp_from_line;
pub use paths::{claude_paths, usage_files};
pub(crate) use paths::{extract_project, extract_session_parts};
pub use resident::ResidentIndex;

pub fn load_entries(shared: &SharedArgs, project_filter: Option<&str>) -> Result<Vec<LoadedEntry>> {
    progress::track_usage_load(progress::UsageLoadAgent("Claude"), shared.json, || {
        load_entries_inner(shared, project_filter)
    })
}

pub fn load_daily_summaries(
    shared: &SharedArgs,
    project_filter: Option<&str>,
    group_by_project: bool,
) -> Result<Vec<UsageSummary>> {
    progress::track_usage_load(progress::UsageLoadAgent("Claude"), shared.json, || {
        daily::load_daily_summaries_inner(shared, project_filter, group_by_project)
    })
}

fn load_entries_inner(
    shared: &SharedArgs,
    project_filter: Option<&str>,
) -> Result<Vec<LoadedEntry>> {
    let paths = claude_paths()?;
    debug_log(
        shared,
        format!(
            "Scanning Claude data directories: {}",
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    let files = usage_files(&paths, project_filter);
    debug_log(shared, format!("Found {} JSONL usage files", files.len()));
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let pricing = if shared.mode == CostMode::Display {
        None
    } else {
        Some(PricingMap::load_with_overrides(
            shared.offline,
            log_level() != Some(0),
            shared.pricing_overrides.iter(),
        ))
    };
    let tz = parse_tz(shared.timezone.as_deref());
    let mode = shared.mode;
    let loaded_files = read_files_parallel(&files, shared.single_thread, |file| {
        read_usage_file(file, tz.as_ref(), mode, pricing.as_ref())
    });
    let loaded_entry_count = loaded_files
        .iter()
        .map(|file| file.entries.len())
        .sum::<usize>();
    debug_log(
        shared,
        format!(
            "Loaded {loaded_entry_count} usage entries from {} JSONL files",
            loaded_files.len()
        ),
    );

    let mut deduped_indexes: FxHashMap<u64, SmallIndexVec> = FxHashMap::default();
    let mut deduped: Vec<LoadedEntry> =
        Vec::with_capacity(loaded_files.iter().map(|file| file.entries.len()).sum());
    for loaded_file in loaded_files {
        for entry in loaded_file.entries {
            if let Some(filter) = project_filter
                && entry.project.as_ref() != filter
            {
                continue;
            }
            push_deduped_entry(entry, &mut deduped_indexes, &mut deduped);
        }
    }
    debug_log(
        shared,
        format!("Kept {} usage entries after deduplication", deduped.len()),
    );
    Ok(deduped)
}

fn usage_token_total(data: &UsageEntry) -> u64 {
    let usage = data.message.usage;
    usage.input_tokens
        + usage.output_tokens
        + usage.cache_creation_token_count()
        + usage.cache_read_input_tokens
}

fn should_replace_deduped_entry(candidate: &UsageEntry, existing: &UsageEntry) -> bool {
    let candidate_is_sidechain = is_sidechain_usage_entry(candidate);
    let existing_is_sidechain = is_sidechain_usage_entry(existing);
    if candidate_is_sidechain != existing_is_sidechain {
        return existing_is_sidechain;
    }

    let candidate_total = usage_token_total(candidate);
    let existing_total = usage_token_total(existing);
    if candidate_total != existing_total {
        return candidate_total > existing_total;
    }

    candidate.message.usage.speed.is_some() && existing.message.usage.speed.is_none()
}

fn push_deduped_entry(
    entry: LoadedEntry,
    deduped_indexes: &mut FxHashMap<u64, SmallIndexVec>,
    deduped: &mut Vec<LoadedEntry>,
) {
    let dedupe_lookup = entry.data.message.id.as_deref().map(|message_id| {
        let request_id = entry.data.request_id.as_deref();
        let exact_hash = usage_dedupe_hash(message_id, request_id);
        let existing_index = deduped_indexes
            .get(&exact_hash)
            .and_then(|indexes| {
                indexes.iter().copied().find(|&index| {
                    loaded_entry_matches_dedupe_key(&deduped[index], message_id, request_id)
                })
            })
            .or_else(|| {
                // /btw sidechain logs can replay parent messages with new request IDs.
                let message_hash = usage_dedupe_hash(message_id, None);
                let candidate_is_sidechain = is_sidechain_usage_entry(&entry.data);
                deduped_indexes.get(&message_hash).and_then(|indexes| {
                    indexes.iter().copied().find(|&index| {
                        loaded_entry_matches_sidechain_dedupe_key(
                            &deduped[index],
                            message_id,
                            candidate_is_sidechain,
                        )
                    })
                })
            });
        (exact_hash, existing_index)
    });

    if let Some((hash, Some(index))) = dedupe_lookup {
        if should_replace_deduped_entry(&entry.data, &deduped[index].data) {
            deduped[index] = entry;
            push_deduped_index(deduped_indexes, hash, index);
            if let Some(message_id) = deduped[index].data.message.id.as_deref() {
                push_deduped_index(deduped_indexes, usage_dedupe_hash(message_id, None), index);
            }
        }
        return;
    }

    let index = deduped.len();
    deduped.push(entry);
    if let Some((hash, None)) = dedupe_lookup {
        push_deduped_index(deduped_indexes, hash, index);
        if let Some(message_id) = deduped[index].data.message.id.as_deref() {
            push_deduped_index(deduped_indexes, usage_dedupe_hash(message_id, None), index);
        }
    }
}

fn usage_dedupe_hash(message_id: &str, request_id: Option<&str>) -> u64 {
    let mut hasher = FxHasher::default();
    message_id.hash(&mut hasher);
    request_id.hash(&mut hasher);
    hasher.finish()
}

fn loaded_entry_matches_dedupe_key(
    entry: &LoadedEntry,
    message_id: &str,
    request_id: Option<&str>,
) -> bool {
    entry.data.message.id.as_deref() == Some(message_id)
        && entry.data.request_id.as_deref() == request_id
}

fn loaded_entry_matches_sidechain_dedupe_key(
    entry: &LoadedEntry,
    message_id: &str,
    candidate_is_sidechain: bool,
) -> bool {
    entry.data.message.id.as_deref() == Some(message_id)
        && (candidate_is_sidechain || is_sidechain_usage_entry(&entry.data))
}

fn is_sidechain_usage_entry(entry: &UsageEntry) -> bool {
    entry.is_sidechain == Some(true)
}

fn push_deduped_index(
    deduped_indexes: &mut FxHashMap<u64, SmallIndexVec>,
    hash: u64,
    index: usize,
) {
    let indexes = deduped_indexes.entry(hash).or_default();
    if !indexes.contains(&index) {
        indexes.push(index);
    }
}

fn read_usage_file(
    path: &Path,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
) -> LoadedFile {
    let (session_id, project_path) = extract_session_parts(path);
    let idents = UsageFileIdents {
        project: Arc::from(extract_project(path)),
        session_id: Arc::from(session_id),
        project_path: Arc::from(project_path),
    };
    let (min_timestamp_ms, raw_entries) = cache::cached_scan(
        path,
        "full",
        scan_usage_bytes,
        write_raw_usage_entry,
        read_raw_usage_entry,
    );
    let mut loaded_file = LoadedFile {
        timestamp: min_timestamp_ms.map(TimestampMs::from_millis),
        entries: Vec::new(),
    };
    for raw_entry in &raw_entries {
        finish_raw_usage_entry(
            raw_entry,
            &idents,
            tz,
            mode,
            pricing,
            &mut loaded_file.entries,
        );
    }
    loaded_file
}

/// Per-source-file identifiers shared by every entry parsed from that file.
struct UsageFileIdents {
    project: Arc<str>,
    session_id: Arc<str>,
    project_path: Arc<str>,
}

/// Timezone/mode/pricing-independent record scanned from one usage line,
/// carrying everything needed to rebuild the original [`UsageEntry`].
#[derive(Debug, Clone, PartialEq)]
struct RawUsageEntry {
    timestamp_ms: i64,
    timestamp: String,
    session_id: Option<String>,
    version: Option<String>,
    usage: TokenUsageRaw,
    model: Option<String>,
    message_id: Option<String>,
    cost_usd: Option<f64>,
    request_id: Option<String>,
    is_api_error_message: Option<bool>,
    is_sidechain: Option<bool>,
    usage_limit_reset_time_ms: Option<i64>,
    advisors: Vec<(String, TokenUsageRaw)>,
}

fn scan_usage_bytes(bytes: &[u8]) -> cache::ScanResult<RawUsageEntry> {
    let usage_marker = memmem::Finder::new(br#""usage""#);
    let mut result = cache::ScanResult::new();
    let mut offset = 0;
    while let Some(newline) = memchr(b'\n', &bytes[offset..]) {
        scan_usage_line(
            &bytes[offset..offset + newline],
            &usage_marker,
            &mut result.min_timestamp_ms,
            &mut result.entries,
        );
        offset += newline + 1;
    }
    if offset < bytes.len() {
        scan_usage_line(
            &bytes[offset..],
            &usage_marker,
            &mut result.tail_min_timestamp_ms,
            &mut result.tail_entries,
        );
    }
    result.consumed = offset as u64;
    result
}

fn scan_usage_line(
    line: &[u8],
    usage_marker: &memmem::Finder,
    min_timestamp_ms: &mut Option<i64>,
    out: &mut Vec<RawUsageEntry>,
) {
    if !has_usage_object(line, usage_marker) {
        return;
    }
    if has_unsupported_null_field(line) {
        return;
    }
    let Ok(data) = serde_json::from_slice::<UsageEntry>(line) else {
        return;
    };
    let Some(timestamp) = parse_ts_timestamp(&data.timestamp) else {
        return;
    };
    *min_timestamp_ms = Some(
        min_timestamp_ms.map_or(timestamp.as_millis(), |current: i64| {
            current.min(timestamp.as_millis())
        }),
    );
    if !is_valid_usage_entry(&data) {
        return;
    }
    let usage_limit_reset_time_ms =
        usage_limit_reset_time_from_line_bytes(line, data.is_api_error_message)
            .map(|reset| reset.as_millis());
    let advisors = advisor_usages_from_line(line)
        .into_iter()
        .map(|advisor| (advisor.model, advisor.usage))
        .collect();
    out.push(RawUsageEntry {
        timestamp_ms: timestamp.as_millis(),
        timestamp: data.timestamp,
        session_id: data.session_id,
        version: data.version,
        usage: data.message.usage,
        model: data.message.model,
        message_id: data.message.id,
        cost_usd: data.cost_usd,
        request_id: data.request_id,
        is_api_error_message: data.is_api_error_message,
        is_sidechain: data.is_sidechain,
        usage_limit_reset_time_ms,
        advisors,
    });
}

/// Keep the compact JSON check cheap while allowing JSON whitespace around
/// the field separator. The typed parser still validates the complete record.
pub(crate) fn has_usage_object(line: &[u8], usage_marker: &memmem::Finder) -> bool {
    usage_marker.find_iter(line).any(|index| {
        let suffix = &line[index + usage_marker.needle().len()..];
        suffix.starts_with(b":{")
            || suffix
                .trim_ascii_start()
                .strip_prefix(b":")
                .is_some_and(|value| value.trim_ascii_start().starts_with(b"{"))
    })
}

fn finish_raw_usage_entry(
    raw: &RawUsageEntry,
    idents: &UsageFileIdents,
    tz: Option<&JiffTimeZone>,
    mode: CostMode,
    pricing: Option<&PricingMap>,
    out: &mut Vec<LoadedEntry>,
) {
    let UsageFileIdents {
        project,
        session_id,
        project_path,
    } = idents;
    let data = UsageEntry {
        session_id: raw.session_id.clone(),
        timestamp: raw.timestamp.clone(),
        version: raw.version.clone(),
        message: UsageMessage {
            usage: raw.usage,
            model: raw.model.clone(),
            id: raw.message_id.clone(),
        },
        cost_usd: raw.cost_usd,
        request_id: raw.request_id.clone(),
        is_api_error_message: raw.is_api_error_message,
        is_sidechain: raw.is_sidechain,
    };
    let timestamp = TimestampMs::from_millis(raw.timestamp_ms);
    let date = format_date_tz(timestamp, tz);
    let cost = calculate_cost(&data, mode, pricing);
    let missing_pricing_model = missing_pricing_model_for_usage(
        data.message.model.as_deref(),
        data.message.usage,
        data.cost_usd,
        mode,
        pricing,
    );
    let usage_limit_reset_time = raw.usage_limit_reset_time_ms.map(TimestampMs::from_millis);
    let model = data.message.model.as_ref().and_then(|model| {
        if model == "<synthetic>" {
            None
        } else if matches!(data.message.usage.speed, Some(Speed::Fast)) {
            Some(suffix_string(model, "-fast"))
        } else {
            Some(model.clone())
        }
    });
    let entry = LoadedEntry {
        data,
        timestamp,
        date,
        project: Arc::clone(project),
        session_id: Arc::clone(session_id),
        project_path: Arc::clone(project_path),
        cost,
        extra_total_tokens: 0,
        credits: None,
        message_count: None,
        model,
        usage_limit_reset_time,
        missing_pricing_model,
    };
    let mut advisor_entries = Vec::new();
    for (index, (advisor_model, advisor_usage)) in raw.advisors.iter().enumerate() {
        let mut advisor_data = entry.data.clone();
        advisor_data.message.id = advisor_data
            .message
            .id
            .map(|message_id| format!("{message_id}:advisor:{index}"));
        advisor_data.message.model = Some(advisor_model.clone());
        advisor_data.message.usage = *advisor_usage;
        advisor_data.cost_usd = None;
        let missing_pricing_model = missing_pricing_model_for_usage(
            Some(advisor_model),
            *advisor_usage,
            None,
            mode,
            pricing,
        );
        advisor_entries.push(LoadedEntry {
            data: advisor_data,
            timestamp,
            date: entry.date.clone(),
            project: Arc::clone(project),
            session_id: Arc::clone(session_id),
            project_path: Arc::clone(project_path),
            cost: calculate_cost_for_usage(
                Some(advisor_model),
                *advisor_usage,
                None,
                mode,
                pricing,
            ),
            extra_total_tokens: 0,
            credits: None,
            message_count: None,
            model: Some(advisor_model.clone()),
            usage_limit_reset_time,
            missing_pricing_model,
        });
    }
    out.push(entry);
    out.extend(advisor_entries);
}

fn write_raw_usage_entry(writer: &mut cache::Writer, entry: &RawUsageEntry) {
    writer.push_i64(entry.timestamp_ms);
    writer.push_str(&entry.timestamp);
    writer.push_opt_str(entry.session_id.as_deref());
    writer.push_opt_str(entry.version.as_deref());
    cache::write_token_usage(writer, entry.usage);
    writer.push_opt_str(entry.model.as_deref());
    writer.push_opt_str(entry.message_id.as_deref());
    writer.push_opt_f64(entry.cost_usd);
    writer.push_opt_str(entry.request_id.as_deref());
    writer.push_opt_bool(entry.is_api_error_message);
    writer.push_opt_bool(entry.is_sidechain);
    writer.push_i64(entry.usage_limit_reset_time_ms.unwrap_or(i64::MIN));
    writer.push_u32(entry.advisors.len() as u32);
    for (model, usage) in &entry.advisors {
        writer.push_str(model);
        cache::write_token_usage(writer, *usage);
    }
}

fn read_raw_usage_entry(reader: &mut cache::Reader<'_>) -> Option<RawUsageEntry> {
    let timestamp_ms = reader.read_i64()?;
    let timestamp = reader.read_str()?;
    let session_id = reader.read_opt_str()?;
    let version = reader.read_opt_str()?;
    let usage = cache::read_token_usage(reader)?;
    let model = reader.read_opt_str()?;
    let message_id = reader.read_opt_str()?;
    let cost_usd = reader.read_opt_f64()?;
    let request_id = reader.read_opt_str()?;
    let is_api_error_message = reader.read_opt_bool()?;
    let is_sidechain = reader.read_opt_bool()?;
    let usage_limit_reset_time_ms = match reader.read_i64()? {
        i64::MIN => None,
        value => Some(value),
    };
    let advisor_count = reader.read_u32()? as usize;
    let mut advisors = Vec::with_capacity(advisor_count.min(1024));
    for _ in 0..advisor_count {
        let model = reader.read_str()?;
        let usage = cache::read_token_usage(reader)?;
        advisors.push((model, usage));
    }
    Some(RawUsageEntry {
        timestamp_ms,
        timestamp,
        session_id,
        version,
        usage,
        model,
        message_id,
        cost_usd,
        request_id,
        is_api_error_message,
        is_sidechain,
        usage_limit_reset_time_ms,
        advisors,
    })
}

#[derive(Debug, Deserialize)]
struct UsageIterationsEnvelope {
    message: UsageIterationsMessage,
}

#[derive(Debug, Deserialize)]
struct UsageIterationsMessage {
    usage: UsageIterations,
}

#[derive(Debug, Deserialize)]
struct UsageIterations {
    #[serde(default)]
    iterations: Vec<UsageIteration>,
}

#[derive(Debug, Deserialize)]
struct UsageIteration {
    #[serde(rename = "type")]
    kind: String,
    model: Option<String>,
    #[serde(flatten)]
    usage: TokenUsageRaw,
}

pub(crate) struct AdvisorUsage {
    model: String,
    usage: TokenUsageRaw,
}

pub(crate) fn advisor_usages_from_line(line: &[u8]) -> Vec<AdvisorUsage> {
    if memmem::find(line, br#""advisor_message""#).is_none() {
        return Vec::new();
    }
    let Ok(envelope) = serde_json::from_slice::<UsageIterationsEnvelope>(line) else {
        return Vec::new();
    };
    envelope
        .message
        .usage
        .iterations
        .into_iter()
        .filter_map(|iteration| {
            (iteration.kind == "advisor_message")
                .then_some(iteration.model)
                .flatten()
                .filter(|model| !model.is_empty())
                .map(|model| AdvisorUsage {
                    model,
                    usage: iteration.usage,
                })
        })
        .collect()
}

fn is_valid_usage_entry(data: &UsageEntry) -> bool {
    if data
        .version
        .as_deref()
        .is_some_and(|version| !is_semver_prefix(version))
    {
        return false;
    }
    if data
        .session_id
        .as_deref()
        .is_some_and(|session_id| session_id.is_empty())
    {
        return false;
    }
    if data
        .request_id
        .as_deref()
        .is_some_and(|request_id| request_id.is_empty())
    {
        return false;
    }
    if data
        .message
        .id
        .as_deref()
        .is_some_and(|message_id| message_id.is_empty())
    {
        return false;
    }
    if data
        .message
        .model
        .as_deref()
        .is_some_and(|model| model.is_empty())
    {
        return false;
    }
    true
}

pub(crate) fn has_unsupported_null_field(line: &[u8]) -> bool {
    for null_index in memmem::find_iter(line, b"null") {
        let Some(before_colon) = line[..null_index].trim_ascii_end().strip_suffix(b":") else {
            continue;
        };
        let Some(before_quote) = before_colon.trim_ascii_end().strip_suffix(b"\"") else {
            continue;
        };
        if let Some(field_start) = memchr::memrchr(b'"', before_quote)
            && is_unsupported_nullable_field(&before_quote[field_start + 1..])
        {
            return true;
        }
    }
    false
}

fn is_unsupported_nullable_field(field: &[u8]) -> bool {
    // Match the raw field bytes directly against the known set of fields that
    // are not allowed to be `null`. Comparing byte slices lets rustc dispatch on
    // length before comparing bytes, which avoids both UTF-8 validation and the
    // hash computation a `phf::Set` lookup would incur on every call.
    matches!(
        field,
        b"id"
            | b"cwd"
            | b"model"
            | b"speed"
            | b"costUSD"
            | b"version"
            | b"sessionId"
            | b"requestId"
            | b"isApiErrorMessage"
            | b"cache_read_input_tokens"
            | b"cache_creation_input_tokens"
    )
}

pub(crate) fn is_semver_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    if !consume_ascii_digits(bytes, &mut index) || bytes.get(index) != Some(&b'.') {
        return false;
    }
    index += 1;
    if !consume_ascii_digits(bytes, &mut index) || bytes.get(index) != Some(&b'.') {
        return false;
    }
    index += 1;
    bytes.get(index).is_some_and(u8::is_ascii_digit)
}

fn consume_ascii_digits(bytes: &[u8], index: &mut usize) -> bool {
    let start = *index;
    while bytes.get(*index).is_some_and(u8::is_ascii_digit) {
        *index += 1;
    }
    *index > start
}

#[doc(hidden)]
pub fn usage_limit_reset_time_from_line(
    line: &str,
    is_api_error_message: Option<bool>,
) -> Option<TimestampMs> {
    usage_limit_reset_time_from_line_bytes(line.as_bytes(), is_api_error_message)
}

fn usage_limit_reset_time_from_line_bytes(
    line: &[u8],
    is_api_error_message: Option<bool>,
) -> Option<TimestampMs> {
    if is_api_error_message != Some(true) {
        return None;
    }
    let marker = b"Claude AI usage limit reached";
    let marker_start = memmem::find(line, marker)?;
    let timestamp_start = memchr::memchr(b'|', &line[marker_start..])? + marker_start + 1;
    let timestamp_end = line[timestamp_start..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .map_or(line.len(), |offset| timestamp_start + offset);
    if timestamp_start == timestamp_end {
        return None;
    }
    let timestamp = std::str::from_utf8(&line[timestamp_start..timestamp_end])
        .ok()?
        .parse::<i64>()
        .ok()?;
    if timestamp <= 0 {
        return None;
    }
    TimestampMs::from_unix_seconds(timestamp)
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use super::{
        RawUsageEntry, extract_session_parts, has_unsupported_null_field,
        paths::is_project_path_segment, push_deduped_entry, read_raw_usage_entry, read_usage_file,
        usage_files, write_raw_usage_entry,
    };
    use crate::{
        LoadedEntry, PricingMap, TimestampMs, TokenUsageRaw, UsageEntry, UsageMessage,
        cache::{Reader, Writer},
        cli::CostMode,
    };
    use turbotokens_test_support::fs_fixture;

    #[test]
    fn roundtrips_raw_usage_entry_cache_encoding() {
        let entries = [
            RawUsageEntry {
                timestamp_ms: 1_775_000_000_000,
                timestamp: "2026-03-29T07:00:00.000Z".to_string(),
                session_id: None,
                version: None,
                usage: TokenUsageRaw {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                    speed: None,
                    cache_creation: None,
                },
                model: None,
                message_id: None,
                cost_usd: None,
                request_id: None,
                is_api_error_message: None,
                is_sidechain: None,
                usage_limit_reset_time_ms: None,
                advisors: Vec::new(),
            },
            RawUsageEntry {
                timestamp_ms: i64::MAX,
                timestamp: "2026-03-29T07:00:00.000+09:00".to_string(),
                session_id: Some("session-β".to_string()),
                version: Some("1.2.3".to_string()),
                usage: TokenUsageRaw {
                    input_tokens: 7,
                    output_tokens: 8,
                    cache_creation_input_tokens: 9,
                    cache_read_input_tokens: 10,
                    speed: Some(crate::Speed::Fast),
                    cache_creation: Some(crate::CacheCreationRaw {
                        ephemeral_5m_input_tokens: 11,
                        ephemeral_1h_input_tokens: 12,
                    }),
                },
                model: Some("claude-opus-4-日本語".to_string()),
                message_id: Some("msg-✓".to_string()),
                cost_usd: Some(0.5),
                request_id: Some("req-1".to_string()),
                is_api_error_message: Some(true),
                is_sidechain: Some(false),
                usage_limit_reset_time_ms: Some(1_775_000_500_000),
                advisors: vec![(
                    "advisor-β".to_string(),
                    TokenUsageRaw {
                        input_tokens: 1,
                        output_tokens: 2,
                        cache_creation_input_tokens: 3,
                        cache_read_input_tokens: 4,
                        speed: None,
                        cache_creation: None,
                    },
                )],
            },
        ];

        let mut writer = Writer::new();
        for entry in &entries {
            write_raw_usage_entry(&mut writer, entry);
        }
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        for entry in &entries {
            let decoded = read_raw_usage_entry(&mut reader).expect("entry decodes");
            assert_eq!(&decoded, entry);
        }
        assert!(reader.finish().is_some());
    }

    #[test]
    fn limits_usage_file_discovery_to_requested_project() {
        let fixture = fs_fixture!({
            "projects/project-a/session-a/a.jsonl": "{}",
            "projects/project-b/session-b/b.jsonl": "{}",
        });

        let files = usage_files(&[fixture.root().to_path_buf()], Some("project-a"));

        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("project-a"));
    }

    #[test]
    fn falls_back_to_full_discovery_for_non_segment_project_filter() {
        let fixture = fs_fixture!({
            "projects/project-a/session-a/a.jsonl": "{}",
            "projects/project-b/session-b/b.jsonl": "{}",
        });

        let files = usage_files(&[fixture.root().to_path_buf()], Some("project-a/session-a"));

        assert_eq!(files.len(), 2);
    }

    #[test]
    fn rejects_dot_segments_as_project_path_segments() {
        assert!(!is_project_path_segment(""));
        assert!(!is_project_path_segment("."));
        assert!(!is_project_path_segment(".."));
        assert!(!is_project_path_segment("project-a/session-a"));
        assert!(!is_project_path_segment("project-a\\session-a"));
        assert!(is_project_path_segment("project-a"));
    }

    #[test]
    fn extracts_file_session_from_modern_claude_project_path() {
        let (session_id, project_path) = extract_session_parts(Path::new(
            "/home/me/.claude/projects/project-a/session-a.jsonl",
        ));

        assert_eq!(session_id, "session-a");
        assert_eq!(project_path, "project-a");
    }

    #[test]
    fn extracts_parent_session_from_nested_claude_project_path() {
        let (session_id, project_path) = extract_session_parts(Path::new(
            "/home/me/.claude/projects/project-a/session-a/chat.jsonl",
        ));

        assert_eq!(session_id, "session-a");
        assert_eq!(project_path, "project-a");
    }

    #[test]
    fn extracts_parent_session_from_claude_subagent_path() {
        let (session_id, project_path) = extract_session_parts(Path::new(
            "/home/me/.claude/projects/project-a/session-a/subagents/worker.jsonl",
        ));

        assert_eq!(session_id, "session-a");
        assert_eq!(project_path, "project-a");
    }

    #[test]
    fn rejects_null_schema_fields_like_typescript_loader() {
        assert!(has_unsupported_null_field(
            br#"{"message":{"usage":{"speed":null}}}"#
        ));
        assert!(has_unsupported_null_field(
            br#"{"message":{"model":null,"usage":{"input_tokens":0}}}"#
        ));
        assert!(has_unsupported_null_field(
            br#"{"sessionId":null,"message":{"usage":{"input_tokens":0}}}"#
        ));
    }

    #[test]
    fn allows_null_content_like_typescript_loader() {
        assert!(!has_unsupported_null_field(
            br#"{"message":{"content":null,"usage":{"input_tokens":0}}}"#
        ));
    }

    #[test]
    fn accepts_whitespace_around_usage_fields() {
        let compact = r#"{"timestamp":"2026-09-08T01:00:00.000Z","version":"2.1.0","message":{"id":"msg-1","model":"claude-sonnet-4","usage":{"input_tokens":123,"output_tokens":45}},"requestId":"req-1","costUSD":0.01}"#;
        for separator in ["\":", "\" : ", "\"\t:\t", "\"\r:\r"] {
            let line = compact.replace("\":", separator);
            for ending in ["", "\n", "\r\n"] {
                let scan = super::scan_usage_bytes(format!("{line}{ending}").as_bytes());
                let entries: Vec<_> = scan.entries.into_iter().chain(scan.tail_entries).collect();

                assert_eq!(
                    entries.len(),
                    1,
                    "separator={separator:?}, ending={ending:?}"
                );
                assert_eq!(entries[0].usage.input_tokens, 123);
                assert_eq!(entries[0].usage.output_tokens, 45);
                assert_eq!(entries[0].cost_usd, Some(0.01));
                assert_eq!(entries[0].message_id.as_deref(), Some("msg-1"));
            }
        }
    }

    #[test]
    fn rejects_null_schema_fields_with_whitespace() {
        for separator in [":", ": ", " \t: \t", "\r:\r"] {
            let line =
                format!(r#"{{"message":{{"model"{separator}null,"usage":{{"input_tokens":0}}}}}}"#);
            assert!(has_unsupported_null_field(line.as_bytes()), "{line}");
            let content = format!(
                r#"{{"message":{{"content"{separator}null,"usage":{{"input_tokens":0}}}}}}"#
            );
            assert!(!has_unsupported_null_field(content.as_bytes()), "{content}");
        }
    }

    #[test]
    fn calculates_advisor_cost_with_the_advisor_model() {
        let fixture = fs_fixture!({
            "projects/project-a/session-a/chat.jsonl": r#"{"timestamp":"2026-05-22T02:34:40.000Z","version":"1.2.3","sessionId":"session-a","message":{"id":"msg-parent","model":"main-model","usage":{"input_tokens":1,"output_tokens":2,"iterations":[{"type":"advisor_message","model":"advisor-model","input_tokens":10,"output_tokens":2,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}]}},"requestId":"req-parent","costUSD":1.23}"#,
        });
        let mut pricing = PricingMap::default();
        pricing.load_json(
            r#"{
                "main-model": {
                    "input_cost_per_token": 100,
                    "output_cost_per_token": 100
                },
                "advisor-model": {
                    "input_cost_per_token": 2,
                    "output_cost_per_token": 3
                }
            }"#,
        );

        let loaded = read_usage_file(
            &fixture.path("projects/project-a/session-a/chat.jsonl"),
            None,
            CostMode::Auto,
            Some(&pricing),
        );

        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].cost, 1.23);
        assert_eq!(loaded.entries[1].model.as_deref(), Some("advisor-model"));
        assert_eq!(loaded.entries[1].cost, 26.0);
    }

    #[test]
    fn keeps_parent_usage_when_sidechain_replays_message_with_new_request_id() {
        let mut deduped_indexes = Default::default();
        let mut deduped = Vec::new();

        push_deduped_entry(
            loaded_usage_entry(UsageEntryFixture {
                message_id: "msg-parent",
                request_id: "req-parent",
                is_sidechain: false,
                cache_read_tokens: 20,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_entry(
            loaded_usage_entry(UsageEntryFixture {
                message_id: "msg-parent",
                request_id: "req-sidechain-replay",
                is_sidechain: true,
                cache_read_tokens: 50_000,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_entry(
            loaded_usage_entry(UsageEntryFixture {
                message_id: "msg-sidechain-answer",
                request_id: "req-sidechain-answer",
                is_sidechain: true,
                cache_read_tokens: 700,
                output_tokens: 30,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].data.message.id.as_deref(), Some("msg-parent"));
        assert_eq!(deduped[0].data.request_id.as_deref(), Some("req-parent"));
        assert_eq!(deduped[0].data.message.usage.cache_read_input_tokens, 20);
        assert_eq!(
            deduped[1].data.message.id.as_deref(),
            Some("msg-sidechain-answer")
        );
        assert_eq!(deduped[1].data.message.usage.cache_read_input_tokens, 700);
    }

    #[test]
    fn refreshes_dedupe_indexes_when_parent_replaces_sidechain_replay() {
        let mut deduped_indexes = Default::default();
        let mut deduped = Vec::new();

        push_deduped_entry(
            loaded_usage_entry(UsageEntryFixture {
                message_id: "msg-parent",
                request_id: "req-sidechain-replay",
                is_sidechain: true,
                cache_read_tokens: 50_000,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_entry(
            loaded_usage_entry(UsageEntryFixture {
                message_id: "msg-parent",
                request_id: "req-parent",
                is_sidechain: false,
                cache_read_tokens: 20,
                output_tokens: 10,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );
        push_deduped_entry(
            loaded_usage_entry(UsageEntryFixture {
                message_id: "msg-parent",
                request_id: "req-parent",
                is_sidechain: false,
                cache_read_tokens: 5,
                output_tokens: 5,
            }),
            &mut deduped_indexes,
            &mut deduped,
        );

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].data.request_id.as_deref(), Some("req-parent"));
        assert_eq!(deduped[0].data.message.usage.cache_read_input_tokens, 20);
    }

    struct UsageEntryFixture {
        message_id: &'static str,
        request_id: &'static str,
        is_sidechain: bool,
        cache_read_tokens: u64,
        output_tokens: u64,
    }

    fn loaded_usage_entry(fixture: UsageEntryFixture) -> LoadedEntry {
        LoadedEntry {
            data: UsageEntry {
                session_id: Some("session-a".to_string()),
                timestamp: "2026-03-29T07:00:00.000Z".to_string(),
                version: Some("1.0.0".to_string()),
                message: UsageMessage {
                    usage: TokenUsageRaw {
                        input_tokens: 0,
                        output_tokens: fixture.output_tokens,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: fixture.cache_read_tokens,
                        speed: None,
                        cache_creation: None,
                    },
                    model: Some("claude-sonnet-4-20250514".to_string()),
                    id: Some(fixture.message_id.to_string()),
                },
                cost_usd: None,
                request_id: Some(fixture.request_id.to_string()),
                is_api_error_message: None,
                is_sidechain: Some(fixture.is_sidechain),
            },
            timestamp: TimestampMs::from_millis(1_775_000_000_000),
            date: "2026-03-29".to_string(),
            project: Arc::from("project-a"),
            session_id: Arc::from("session-a"),
            project_path: Arc::from("project-a"),
            cost: 0.0,
            extra_total_tokens: 0,
            credits: None,
            message_count: None,
            model: Some("claude-sonnet-4-20250514".to_string()),
            usage_limit_reset_time: None,
            missing_pricing_model: None,
        }
    }
}
