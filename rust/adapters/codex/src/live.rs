use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Duration,
};

use jiff::tz::TimeZone as JiffTimeZone;
use serde_json::json;

use turbotokens_adapter_common::live::{
    Alert, AlertState, AlertThresholds, Burn, ByteCursor, Dashboard, DashboardView, LiveBook,
    LiveEvent, LiveMetrics, LiveOutput, MetricsServer, TokenTotals, detect_output,
    map_stream_result, read_appended, write_human_line, write_json_line,
};

use crate::{
    PricingMap, Result, TokenUsageRaw, calculate_cost_for_usage,
    cli::{CostMode, LiveArgs, SharedArgs},
    format_date_tz, json_float, log_level, non_cached_input_tokens, parse_ts_timestamp, parse_tz,
    utc_now,
};

use super::parser::{
    CodexLineKind, add_codex_exec_event, add_codex_exec_event_from_value, codex_line_usage_kind,
    codex_session_id, file_modified_timestamp, visit_codex_session_entry,
};
use super::paths::{CodexUsageSource, codex_usage_sources, collect_deduped_codex_usage_files};
use super::types::{CodexLogEntry, CodexSessionLogEntry};
use super::{CodexRawUsage, CodexServiceTier, CodexTokenUsageEvent};

const WEBHOOK_TIMEOUT_SECONDS: u64 = 5;

/// Streams Codex token usage as it is appended to the session JSONL logs,
/// either as NDJSON events (`--json`), one human-readable line per event
/// (piped stdout), or an in-place terminal dashboard (TTY). Mirrors the Claude
/// live loop, including threshold alerts and the Prometheus endpoint.
pub fn run_live(args: &LiveArgs) -> Result<()> {
    let shared = &args.shared;
    let sources = codex_usage_sources()?;
    let mut state = CodexLiveState::new(
        shared,
        AlertThresholds {
            cost: args.alert_cost,
            tokens: args.alert_tokens,
        },
    );
    let output_mode = detect_output(shared.json);
    let metrics_server = match &args.serve {
        Some(addr) => Some(MetricsServer::start(addr)?),
        None => None,
    };

    // Seed from the existing logs, then only appended bytes are scanned.
    let mut events = Vec::new();
    for group in collect_deduped_codex_usage_files(&sources) {
        for file in &group.files {
            if let Ok(bytes) = fs::read(file) {
                state.feed_bytes(&group.dir, file, &bytes, &mut events);
            }
        }
    }
    state.live = true;

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut dashboard = Dashboard::default();
    let startup = emit_startup(
        output_mode,
        shared,
        &mut state,
        &sources,
        args.interval_ms,
        &events,
        &mut dashboard,
        &mut out,
    );
    if !map_stream_result(startup)? {
        return Ok(());
    }
    deliver_alerts(
        output_mode,
        &state.check_alerts(),
        args.webhook.as_deref(),
        &mut state,
        &mut out,
    )?;
    update_metrics(&metrics_server, &mut state);

    let interval = Duration::from_millis(args.interval_ms);
    loop {
        thread::sleep(interval);
        state.refresh_today();
        events.clear();
        for group in collect_deduped_codex_usage_files(&sources) {
            for file in &group.files {
                let Ok(metadata) = fs::metadata(file) else {
                    continue;
                };
                state.poll_file(&group.dir, file, metadata.len(), &mut events);
            }
        }
        deliver_alerts(
            output_mode,
            &state.check_alerts(),
            args.webhook.as_deref(),
            &mut state,
            &mut out,
        )?;
        let tick = emit_tick(
            output_mode,
            shared,
            &mut state,
            &sources,
            args.interval_ms,
            &events,
            &mut dashboard,
            &mut out,
        );
        if !map_stream_result(tick)? {
            return Ok(());
        }
        update_metrics(&metrics_server, &mut state);
    }
}

/// Fires the delivery side of freshly-triggered alerts: an optional webhook
/// POST plus the mode-appropriate visible line.
fn deliver_alerts(
    output_mode: LiveOutput,
    alerts: &[Alert],
    webhook: Option<&str>,
    state: &mut CodexLiveState,
    out: &mut impl Write,
) -> Result<()> {
    for alert in alerts {
        if let Some(url) = webhook {
            post_webhook(url, &alert.to_json());
        }
        match output_mode {
            // NDJSON stays machine-clean on stdout; alerts go to stderr.
            LiveOutput::Json => eprintln!("{}", alert.to_json()),
            LiveOutput::Human => {
                map_stream_result(writeln!(out, "{}", alert.banner()).and_then(|()| out.flush()))?;
            }
            LiveOutput::Dashboard => state.alert_banner = Some(alert.banner()),
        }
    }
    Ok(())
}

/// POSTs the alert payload with a short timeout; a failing webhook must never
/// take the live loop down.
fn post_webhook(url: &str, body: &serde_json::Value) {
    let result = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(WEBHOOK_TIMEOUT_SECONDS)))
        .build()
        .new_agent()
        .post(url)
        .header("Content-Type", "application/json")
        .send(body.to_string());
    if let Err(error) = result {
        eprintln!("turbotokens live: webhook POST to {url} failed: {error}");
    }
}

fn update_metrics(server: &Option<MetricsServer>, state: &mut CodexLiveState) {
    if let Some(server) = server {
        server.update(turbotokens_adapter_common::live::render_prometheus(
            &state.live_metrics(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_startup(
    output_mode: LiveOutput,
    shared: &SharedArgs,
    state: &mut CodexLiveState,
    sources: &[CodexUsageSource],
    interval_ms: u64,
    events: &[LiveEvent],
    dashboard: &mut Dashboard,
    out: &mut impl Write,
) -> io::Result<()> {
    match output_mode {
        LiveOutput::Json => {
            write_json_line(out, &state.snapshot_json())?;
            for event in events {
                write_json_line(out, &event.to_json())?;
            }
        }
        LiveOutput::Human => {
            for event in events {
                write_human_line(out, event)?;
            }
        }
        LiveOutput::Dashboard => {
            dashboard.render(shared, &state.dashboard_view(sources, interval_ms), out)?;
        }
    }
    out.flush()
}

#[allow(clippy::too_many_arguments)]
fn emit_tick(
    output_mode: LiveOutput,
    shared: &SharedArgs,
    state: &mut CodexLiveState,
    sources: &[CodexUsageSource],
    interval_ms: u64,
    events: &[LiveEvent],
    dashboard: &mut Dashboard,
    out: &mut impl Write,
) -> io::Result<()> {
    match output_mode {
        LiveOutput::Json => {
            for event in events {
                write_json_line(out, &event.to_json())?;
            }
        }
        LiveOutput::Human => {
            for event in events {
                write_human_line(out, event)?;
            }
        }
        LiveOutput::Dashboard => {
            if dashboard.should_render(!events.is_empty()) {
                dashboard.render(shared, &state.dashboard_view(sources, interval_ms), out)?;
            }
        }
    }
    out.flush()
}

/// Per-file tail state: the byte cursor plus the running parser context a
/// token_count delta is resolved against.
#[derive(Default)]
struct CodexFileState {
    cursor: ByteCursor,
    session_id: String,
    project: Arc<str>,
    previous_totals: Option<CodexRawUsage>,
    current_model: Option<String>,
    current_model_is_fallback: bool,
    current_service_tier: Option<CodexServiceTier>,
    fallback_timestamp: String,
}

struct CodexLiveState {
    tz: Option<JiffTimeZone>,
    mode: CostMode,
    pricing: Option<PricingMap>,
    cursors: crate::fast::FxHashMap<PathBuf, CodexFileState>,
    /// Every accepted event, kept so the "today" bucket can be rebuilt at
    /// midnight in the configured timezone.
    accepted: Vec<LiveEvent>,
    book: LiveBook,
    burn: Burn,
    alert_state: AlertState,
    /// Latest alert banner shown as a dashboard row; sticky once fired.
    alert_banner: Option<String>,
    /// False while seeding from existing logs: historical entries must not
    /// count toward the live burn rate.
    live: bool,
}

impl CodexLiveState {
    fn new(shared: &SharedArgs, thresholds: AlertThresholds) -> Self {
        let tz = parse_tz(shared.timezone.as_deref());
        let pricing = if shared.mode == CostMode::Display {
            None
        } else {
            Some(PricingMap::load_with_overrides(
                shared.offline,
                log_level() != Some(0),
                shared.pricing_overrides.iter(),
            ))
        };
        let today = format_date_tz(utc_now(), tz.as_ref());
        Self {
            tz,
            mode: shared.mode,
            pricing,
            cursors: crate::fast::FxHashMap::default(),
            accepted: Vec::new(),
            book: LiveBook::new(today),
            burn: Burn::default(),
            alert_state: AlertState::new(thresholds),
            alert_banner: None,
            live: false,
        }
    }

    fn feed_bytes(&mut self, dir: &Path, path: &Path, bytes: &[u8], events: &mut Vec<LiveEvent>) {
        let mut parsed = Vec::new();
        let project = {
            let file = self
                .cursors
                .entry(path.to_path_buf())
                .or_insert_with(|| codex_file_state(dir, path));
            feed_file(file, bytes, &mut parsed);
            Arc::clone(&file.project)
        };
        for event in parsed {
            self.accept_event(event, &project, events);
        }
    }

    fn poll_file(&mut self, dir: &Path, path: &Path, size: u64, events: &mut Vec<LiveEvent>) {
        let position = self.cursors.get(path).map(|file| file.cursor.position());
        match position {
            // New session file: scan it from the start.
            None => {
                if let Ok(bytes) = fs::read(path) {
                    self.feed_bytes(dir, path, &bytes, events);
                }
            }
            // Shrunk (rotated or rewritten): reset and rescan from offset 0.
            Some(position) if size < position => {
                self.cursors.remove(path);
                if let Ok(bytes) = fs::read(path) {
                    self.feed_bytes(dir, path, &bytes, events);
                }
            }
            Some(position) if size > position => {
                if let Some(bytes) = read_appended(path, position) {
                    self.feed_bytes(dir, path, &bytes, events);
                }
            }
            _ => {}
        }
    }

    /// Maps a parsed token_count event onto the same token split the Codex
    /// report path uses: non-cached input, cached input as cache reads, and
    /// output with reasoning already included.
    fn accept_event(
        &mut self,
        event: CodexTokenUsageEvent,
        project: &Arc<str>,
        events: &mut Vec<LiveEvent>,
    ) {
        let Some(timestamp) = parse_ts_timestamp(&event.timestamp) else {
            return;
        };
        let usage = TokenUsageRaw {
            input_tokens: non_cached_input_tokens(event.input_tokens, event.cached_input_tokens),
            output_tokens: event.output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: event.cached_input_tokens,
            speed: None,
            cache_creation: None,
        };
        let cost = calculate_cost_for_usage(
            event.model.as_deref(),
            usage,
            None,
            self.mode,
            self.pricing.as_ref(),
        );
        let live_event = LiveEvent {
            timestamp_ms: timestamp.as_millis(),
            date: format_date_tz(timestamp, self.tz.as_ref()),
            project: Arc::clone(project),
            session_id: Arc::from(event.session_id.as_str()),
            model: event.model,
            usage,
            cost,
            agent: "codex",
        };
        self.book.add_contribution(&live_event);
        if self.live {
            self.burn.push(live_event.total_tokens());
        }
        self.book.push_recent(live_event.clone());
        self.accepted.push(live_event.clone());
        events.push(live_event);
    }

    /// Edge-triggered threshold alerts against today's totals.
    fn check_alerts(&mut self) -> Vec<Alert> {
        self.alert_state.check(
            &self.book.today,
            self.book.today_totals.cost,
            self.book.today_totals.total(),
        )
    }

    fn live_metrics(&mut self) -> LiveMetrics {
        LiveMetrics {
            input_tokens: self.book.today_totals.input_tokens,
            output_tokens: self.book.today_totals.output_tokens,
            cache_creation_tokens: self.book.today_totals.cache_creation_tokens,
            cache_read_tokens: self.book.today_totals.cache_read_tokens,
            cost_usd: self.book.today_totals.cost,
            tokens_per_minute: self.burn.rate(),
            model_tokens: self
                .book
                .model_totals
                .iter()
                .map(|(model, totals)| (model.clone(), totals.total()))
                .collect(),
            sessions_active: self.book.sessions_active(),
            files_watched: self.cursors.len() as u64,
        }
    }

    fn dashboard_view(
        &mut self,
        sources: &[CodexUsageSource],
        interval_ms: u64,
    ) -> DashboardView<'_> {
        let burn_sparkline = self.burn.sparkline(12);
        let burn_rate = self.burn.rate();
        DashboardView {
            dirs: sources
                .iter()
                .map(|source| source.dir.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            interval_ms,
            files_watched: self.cursors.len(),
            today: &self.book.today,
            today_totals: &self.book.today_totals,
            burn_rate,
            burn_sparkline,
            models: self
                .book
                .model_totals
                .iter()
                .map(|(model, totals)| (model.clone(), totals.clone()))
                .collect(),
            sessions: self.book.session_views(),
            recent: &self.book.recent,
            alert_banner: self.alert_banner.as_deref(),
        }
    }

    /// Roll the "today" bucket over at midnight in the configured timezone.
    fn refresh_today(&mut self) {
        let today = format_date_tz(utc_now(), self.tz.as_ref());
        if today == self.book.today {
            return;
        }
        let mut today_totals = TokenTotals::default();
        let mut model_totals = crate::fast::FxHashMap::<String, TokenTotals>::default();
        for event in &self.accepted {
            if event.date != today {
                continue;
            }
            today_totals.add(event.usage, event.cost);
            if let Some(model) = &event.model {
                model_totals
                    .entry(model.clone())
                    .or_default()
                    .add(event.usage, event.cost);
            }
        }
        self.book.today = today;
        self.book.today_totals = today_totals;
        self.book.model_totals = model_totals;
    }

    fn snapshot_json(&self) -> serde_json::Value {
        json!({
            "type": "snapshot",
            "agent": "codex",
            "date": self.book.today,
            "files": self.cursors.len(),
            "inputTokens": self.book.today_totals.input_tokens,
            "outputTokens": self.book.today_totals.output_tokens,
            "cacheCreationTokens": self.book.today_totals.cache_creation_tokens,
            "cacheReadTokens": self.book.today_totals.cache_read_tokens,
            "totalTokens": self.book.today_totals.total(),
            "cost": json_float(self.book.today_totals.cost),
        })
    }
}

/// Splits a file into complete lines and runs the session/headless token_count
/// parsers over them, keeping the per-file parser context between feeds.
fn feed_file(file: &mut CodexFileState, bytes: &[u8], parsed: &mut Vec<CodexTokenUsageEvent>) {
    let CodexFileState {
        cursor,
        session_id,
        previous_totals,
        current_model,
        current_model_is_fallback,
        current_service_tier,
        fallback_timestamp,
        ..
    } = file;
    cursor.feed(bytes, |line| match codex_line_usage_kind(line) {
        Some(CodexLineKind::Session) => {
            if let Ok(value) = serde_json::from_slice::<CodexSessionLogEntry<'_>>(line) {
                let _ = visit_codex_session_entry(
                    session_id,
                    value,
                    previous_totals,
                    current_model,
                    current_model_is_fallback,
                    current_service_tier,
                    &mut |event| {
                        parsed.push(event);
                        Ok(())
                    },
                );
            }
        }
        Some(CodexLineKind::Headless) => {
            if let Ok(value) = serde_json::from_slice::<CodexLogEntry<'_>>(line) {
                let _ = add_codex_exec_event(
                    session_id,
                    &value,
                    fallback_timestamp,
                    current_model,
                    current_model_is_fallback,
                    &mut |event| {
                        parsed.push(event);
                        Ok(())
                    },
                );
            } else {
                let _ = add_codex_exec_event_from_value(
                    session_id,
                    line,
                    fallback_timestamp,
                    current_model,
                    current_model_is_fallback,
                    &mut |event| {
                        parsed.push(event);
                        Ok(())
                    },
                );
            }
        }
        None => {}
    });
}

fn codex_file_state(dir: &Path, path: &Path) -> CodexFileState {
    let session_id = codex_session_id(dir, path);
    let project = session_id
        .rfind('/')
        .map(|index| session_id[..index].to_string())
        .unwrap_or_else(|| "codex".to_string());
    CodexFileState {
        cursor: ByteCursor::default(),
        session_id,
        project: Arc::from(project),
        previous_totals: None,
        current_model: None,
        current_model_is_fallback: false,
        current_service_tier: None,
        fallback_timestamp: file_modified_timestamp(path),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use turbotokens_test_support::fs_fixture;

    use super::*;

    fn live_state() -> CodexLiveState {
        let mut state = CodexLiveState::new(
            &SharedArgs {
                mode: CostMode::Calculate,
                timezone: Some("UTC".to_string()),
                offline: true,
                ..SharedArgs::default()
            },
            AlertThresholds::default(),
        );
        // Match the fixed timestamp token_count_line stamps on its entries.
        state.book.today = "2026-07-27".to_string();
        state
    }

    fn token_count_line(input: u64, cached: u64, output: u64, reasoning: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-07-27T18:00:00.000Z","type":"event_msg","payload":{{"type":"token_count","info":{{"model":"gpt-5","last_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"reasoning_output_tokens":{reasoning},"total_tokens":{}}}}}}}}}"#,
            input + output
        )
    }

    #[test]
    fn maps_token_count_lines_like_the_report_path() {
        let dir = Path::new("/tmp/codex/sessions");
        let path = Path::new("/tmp/codex/sessions/session-a.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();

        state.feed_bytes(
            dir,
            path,
            format!("{}\n", token_count_line(100, 10, 50, 5)).as_bytes(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.session_id.as_ref(), "session-a");
        assert_eq!(event.project.as_ref(), "codex");
        assert_eq!(event.model.as_deref(), Some("gpt-5"));
        assert_eq!(event.agent, "codex");
        // input = input - cached, cacheRead = cached, output keeps reasoning.
        assert_eq!(event.usage.input_tokens, 90);
        assert_eq!(event.usage.cache_read_input_tokens, 10);
        assert_eq!(event.usage.output_tokens, 50);
        assert_eq!(event.total_tokens(), 150);
        assert_eq!(state.book.today_totals.total(), 150);
    }

    #[test]
    fn buffers_a_partial_line_until_its_newline_arrives() {
        let dir = Path::new("/tmp/codex/sessions");
        let path = Path::new("/tmp/codex/sessions/session-a.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let line = token_count_line(100, 10, 50, 5);
        let split = line.len() / 2;

        state.feed_bytes(dir, path, line.as_bytes()[..split].as_ref(), &mut events);
        assert!(events.is_empty());

        state.feed_bytes(
            dir,
            path,
            format!("{}\n", &line[split..]).as_bytes(),
            &mut events,
        );
        assert_eq!(events.len(), 1);
        let cursor = state.cursors.get(path).unwrap();
        assert_eq!(cursor.cursor.offset as usize, line.len() + 1);
        assert!(cursor.cursor.tail.is_empty());
    }

    #[test]
    fn deltas_cumulative_total_token_usage_like_the_report_path() {
        let dir = Path::new("/tmp/codex/sessions");
        let path = Path::new("/tmp/codex/sessions/session-a.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let cumulative = |input: u64, output: u64| {
            format!(
                r#"{{"timestamp":"2026-07-27T18:00:00.000Z","type":"event_msg","payload":{{"type":"token_count","info":{{"model":"gpt-5","total_token_usage":{{"input_tokens":{input},"cached_input_tokens":0,"output_tokens":{output},"reasoning_output_tokens":0,"total_tokens":{}}}}}}}}}"#,
                input + output
            )
        };

        state.feed_bytes(
            dir,
            path,
            format!("{}\n{}\n", cumulative(100, 50), cumulative(160, 80)).as_bytes(),
            &mut events,
        );

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].usage.input_tokens, 100);
        assert_eq!(events[1].usage.input_tokens, 60);
        assert_eq!(events[1].usage.output_tokens, 30);
        assert_eq!(state.book.today_totals.total(), 150 + 90);
    }

    #[test]
    fn picks_up_appends_through_poll_file() {
        let fixture = fs_fixture!({
            "sessions/session-a.jsonl": format!("{}\n", token_count_line(100, 10, 50, 5)),
        });
        let dir = fixture.path("sessions");
        let path = fixture.path("sessions/session-a.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let size = fs::metadata(&path).unwrap().len();
        state.poll_file(&dir, &path, size, &mut events);
        assert_eq!(events.len(), 1);

        use std::io::Write as _;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", token_count_line(40, 0, 10, 0)).unwrap();
        let size = fs::metadata(&path).unwrap().len();
        state.poll_file(&dir, &path, size, &mut events);

        assert_eq!(events.len(), 2);
        assert_eq!(events[1].usage.input_tokens, 40);
        assert_eq!(state.book.today_totals.total(), 150 + 50);
    }

    #[test]
    fn fires_a_token_alert_once_when_todays_totals_cross() {
        let dir = Path::new("/tmp/codex/sessions");
        let path = Path::new("/tmp/codex/sessions/session-a.jsonl");
        let mut state = live_state();
        state.alert_state = AlertState::new(AlertThresholds {
            cost: None,
            tokens: Some(100),
        });
        let mut events = Vec::new();

        state.feed_bytes(
            dir,
            path,
            format!("{}\n", token_count_line(100, 10, 50, 5)).as_bytes(),
            &mut events,
        );

        let fired = state.check_alerts();
        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].metric,
            turbotokens_adapter_common::live::AlertMetric::Tokens
        );
        assert_eq!(fired[0].value, 150.0);
        assert!(state.check_alerts().is_empty());
    }

    #[test]
    fn rolls_today_totals_over_with_the_configured_timezone() {
        let mut state = live_state();
        state.book.today = "1999-01-01".to_string();
        let mut events = Vec::new();
        state.feed_bytes(
            Path::new("/tmp/codex/sessions"),
            Path::new("/tmp/codex/sessions/session-a.jsonl"),
            format!("{}\n", token_count_line(100, 10, 50, 5)).as_bytes(),
            &mut events,
        );
        // The seeded entry is dated 2026-07-27, not the stale "today".
        assert_eq!(state.book.today_totals.total(), 0);

        state.refresh_today();

        assert_ne!(state.book.today, "1999-01-01");
        let expected = if state.book.today == "2026-07-27" {
            150
        } else {
            0
        };
        assert_eq!(state.book.today_totals.total(), expected);
    }
}
