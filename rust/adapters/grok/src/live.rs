use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use serde_json::json;

use turbotokens_adapter_common::live::{
    Alert, AlertState, AlertThresholds, Burn, ByteCursor, Dashboard, DashboardView, LiveBook,
    LiveEvent, LiveMetrics, LiveOutput, MetricsServer, TokenTotals, detect_output,
    map_stream_result, read_appended, write_human_line, write_json_line,
};

use crate::{
    PricingMap, Result,
    cli::{LiveArgs, SharedArgs},
    format_date_tz, json_float, log_level, parse_tz, utc_now,
};

use super::parser::{
    GrokUsageEntry, ModelTimelines, grok_entry_key, grok_entry_to_loaded, record_model_event,
    turn_event_from_bytes, usage_line_from_bytes,
};
use super::paths::{data_dirs, discover_event_files, discover_usage_logs};

const WEBHOOK_TIMEOUT_SECONDS: u64 = 5;

/// Streams Grok Build token usage as it is appended to `logs/unified.jsonl`,
/// using per-session `events.jsonl` files for model attribution. Same output
/// modes as the Claude and Codex live loops: NDJSON, human lines, or a TTY
/// dashboard, plus optional threshold alerts and Prometheus metrics.
pub fn run_live(args: &LiveArgs) -> Result<()> {
    let shared = &args.shared;
    let mut state = GrokLiveState::new(
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

    let mut events = Vec::new();
    for file in discover_event_files()? {
        if let Ok(bytes) = fs::read(&file) {
            state.feed_bytes(&file, &bytes, &mut events);
        }
    }
    for file in discover_usage_logs() {
        if let Ok(bytes) = fs::read(&file) {
            state.feed_bytes(&file, &bytes, &mut events);
        }
    }
    state.live = true;

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut dashboard = Dashboard::default();
    let dirs = data_dirs();
    let startup = emit_startup(
        output_mode,
        shared,
        &mut state,
        &dirs,
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
        for file in discover_event_files()? {
            let Ok(metadata) = fs::metadata(&file) else {
                continue;
            };
            state.poll_file(&file, metadata.len(), &mut events);
        }
        for file in discover_usage_logs() {
            let Ok(metadata) = fs::metadata(&file) else {
                continue;
            };
            state.poll_file(&file, metadata.len(), &mut events);
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
            &dirs,
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

fn deliver_alerts(
    output_mode: LiveOutput,
    alerts: &[Alert],
    webhook: Option<&str>,
    state: &mut GrokLiveState,
    out: &mut impl Write,
) -> Result<()> {
    for alert in alerts {
        if let Some(url) = webhook {
            post_webhook(url, &alert.to_json());
        }
        match output_mode {
            LiveOutput::Json => eprintln!("{}", alert.to_json()),
            LiveOutput::Human => {
                map_stream_result(writeln!(out, "{}", alert.banner()).and_then(|()| out.flush()))?;
            }
            LiveOutput::Dashboard => state.alert_banner = Some(alert.banner()),
        }
    }
    Ok(())
}

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

fn update_metrics(server: &Option<MetricsServer>, state: &mut GrokLiveState) {
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
    state: &mut GrokLiveState,
    dirs: &[PathBuf],
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
            dashboard.render(shared, &state.dashboard_view(dirs, interval_ms), out)?;
        }
    }
    out.flush()
}

#[allow(clippy::too_many_arguments)]
fn emit_tick(
    output_mode: LiveOutput,
    shared: &SharedArgs,
    state: &mut GrokLiveState,
    dirs: &[PathBuf],
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
                dashboard.render(shared, &state.dashboard_view(dirs, interval_ms), out)?;
            }
        }
    }
    out.flush()
}

#[derive(Clone, Copy)]
enum WatchedKind {
    Usage,
    Events,
}

struct GrokFileState {
    cursor: ByteCursor,
    kind: WatchedKind,
}

struct GrokLiveState {
    tz: Option<jiff::tz::TimeZone>,
    mode: crate::cli::CostMode,
    pricing: PricingMap,
    cursors: HashMap<PathBuf, GrokFileState>,
    timelines: ModelTimelines,
    seen: HashSet<String>,
    accepted: Vec<LiveEvent>,
    book: LiveBook,
    burn: Burn,
    alert_state: AlertState,
    alert_banner: Option<String>,
    live: bool,
}

impl GrokLiveState {
    fn new(shared: &SharedArgs, thresholds: AlertThresholds) -> Self {
        let tz = parse_tz(shared.timezone.as_deref());
        let pricing = PricingMap::load_with_overrides(
            shared.offline,
            log_level() != Some(0),
            shared.pricing_overrides.iter(),
        );
        let today = format_date_tz(utc_now(), tz.as_ref());
        Self {
            tz,
            mode: shared.mode,
            pricing,
            cursors: HashMap::new(),
            timelines: HashMap::new(),
            seen: HashSet::new(),
            accepted: Vec::new(),
            book: LiveBook::new(today),
            burn: Burn::default(),
            alert_state: AlertState::new(thresholds),
            alert_banner: None,
            live: false,
        }
    }

    fn feed_bytes(&mut self, path: &Path, bytes: &[u8], events: &mut Vec<LiveEvent>) {
        let kind = watched_kind(path);
        let mut usage_lines = Vec::new();
        let mut model_events = Vec::new();
        {
            let file = self
                .cursors
                .entry(path.to_path_buf())
                .or_insert_with(|| GrokFileState {
                    cursor: ByteCursor::default(),
                    kind,
                });
            file.cursor.feed(bytes, |line| match file.kind {
                WatchedKind::Events => {
                    if let Some(event) = turn_event_from_bytes(line) {
                        model_events.push(event);
                    }
                }
                WatchedKind::Usage => usage_lines.push(line.to_vec()),
            });
        }
        for (session_id, event) in model_events {
            record_model_event(&mut self.timelines, session_id, event);
        }
        for line in usage_lines {
            if let Some(entry) = usage_line_from_bytes(&line, &self.timelines) {
                self.accept_usage(entry, events);
            }
        }
    }

    fn poll_file(&mut self, path: &Path, size: u64, events: &mut Vec<LiveEvent>) {
        let position = self.cursors.get(path).map(|file| file.cursor.position());
        match position {
            None => {
                if let Ok(bytes) = fs::read(path) {
                    self.feed_bytes(path, &bytes, events);
                }
            }
            Some(position) if size < position => {
                self.cursors.remove(path);
                if let Ok(bytes) = fs::read(path) {
                    self.feed_bytes(path, &bytes, events);
                }
            }
            Some(position) if size > position => {
                if let Some(bytes) = read_appended(path, position) {
                    self.feed_bytes(path, &bytes, events);
                }
            }
            _ => {}
        }
    }

    fn accept_usage(&mut self, entry: GrokUsageEntry, events: &mut Vec<LiveEvent>) {
        if !self.seen.insert(grok_entry_key(&entry)) {
            return;
        }
        let loaded = grok_entry_to_loaded(entry, self.tz.as_ref(), self.mode, &self.pricing);
        let mut usage = loaded.data.message.usage;
        // Grok reports keep reasoning in extra_total_tokens; live folds it into
        // output so totalTokens and dashboard burn match the report totals.
        usage.output_tokens = usage
            .output_tokens
            .saturating_add(loaded.extra_total_tokens);
        let live_event = LiveEvent {
            timestamp_ms: loaded.timestamp.as_millis(),
            date: loaded.date,
            project: loaded.project,
            session_id: loaded.session_id,
            model: loaded.model,
            usage,
            cost: loaded.cost,
        };
        self.book.add_contribution(&live_event);
        if self.live {
            self.burn.push(live_event.total_tokens());
        }
        self.book.push_recent(live_event.clone());
        self.accepted.push(live_event.clone());
        events.push(live_event);
    }

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

    fn dashboard_view(&mut self, dirs: &[PathBuf], interval_ms: u64) -> DashboardView<'_> {
        let burn_sparkline = self.burn.sparkline(12);
        let burn_rate = self.burn.rate();
        DashboardView {
            dirs: dirs
                .iter()
                .map(|path| path.display().to_string())
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

fn watched_kind(path: &Path) -> WatchedKind {
    if path.file_name().and_then(|name| name.to_str()) == Some("events.jsonl") {
        WatchedKind::Events
    } else {
        WatchedKind::Usage
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use turbotokens_test_support::fs_fixture;

    use super::*;
    use crate::cli::CostMode;

    fn live_state() -> GrokLiveState {
        let mut state = GrokLiveState::new(
            &SharedArgs {
                mode: CostMode::Calculate,
                timezone: Some("UTC".to_string()),
                offline: true,
                ..SharedArgs::default()
            },
            AlertThresholds::default(),
        );
        state.book.today = "2026-07-08".to_string();
        state
    }

    fn inference_line(
        session: &str,
        prompt: u64,
        cached: u64,
        completion: u64,
        reasoning: u64,
    ) -> String {
        format!(
            r#"{{"ts":"2026-07-08T07:10:13.766Z","src":"shell","sid":"{session}","msg":"shell.turn.inference_done","ctx":{{"loop_index":1,"prompt_tokens":{prompt},"cached_prompt_tokens":{cached},"completion_tokens":{completion},"reasoning_tokens":{reasoning}}}}}"#
        )
    }

    fn turn_started(session: &str, model: &str) -> String {
        format!(
            r#"{{"type":"turn_started","ts":"2026-07-08T07:00:00.000Z","session_id":"{session}","model_id":"{model}","turn_number":0}}"#
        )
    }

    #[test]
    fn maps_inference_done_with_the_active_turn_model() {
        let mut state = live_state();
        let mut events = Vec::new();
        state.feed_bytes(
            Path::new("/tmp/grok/sessions/ws/session-a/events.jsonl"),
            format!("{}\n", turn_started("session-a", "grok-4.5")).as_bytes(),
            &mut events,
        );
        assert!(events.is_empty());

        state.feed_bytes(
            Path::new("/tmp/grok/logs/unified.jsonl"),
            format!("{}\n", inference_line("session-a", 36_458, 32_876, 251, 99)).as_bytes(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.session_id.as_ref(), "session-a");
        assert_eq!(event.project.as_ref(), "grok");
        assert_eq!(event.model.as_deref(), Some("grok-4.5"));
        assert_eq!(event.usage.input_tokens, 3_582);
        assert_eq!(event.usage.cache_read_input_tokens, 32_876);
        assert_eq!(event.usage.output_tokens, 251 + 99);
        assert_eq!(event.total_tokens(), 36_808);
        assert!((event.cost - 0.019_126_8).abs() < f64::EPSILON);
        assert_eq!(state.book.today_totals.total(), 36_808);
    }

    #[test]
    fn keeps_unknown_model_when_turn_events_are_missing() {
        let mut state = live_state();
        let mut events = Vec::new();
        state.feed_bytes(
            Path::new("/tmp/grok/logs/unified.jsonl"),
            format!("{}\n", inference_line("session-a", 10, 0, 2, 0)).as_bytes(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model.as_deref(), Some("unknown"));
        assert_eq!(events[0].usage.input_tokens, 10);
        assert_eq!(events[0].usage.output_tokens, 2);
    }

    #[test]
    fn buffers_a_partial_line_until_its_newline_arrives() {
        let path = Path::new("/tmp/grok/logs/unified.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let line = inference_line("session-a", 10, 0, 2, 0);
        let split = line.len() / 2;

        state.feed_bytes(path, line.as_bytes()[..split].as_ref(), &mut events);
        assert!(events.is_empty());

        state.feed_bytes(
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
    fn skips_non_usage_lines_and_duplicate_keys() {
        let path = Path::new("/tmp/grok/logs/unified.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let usage = inference_line("session-a", 10, 0, 2, 0);
        state.feed_bytes(
            path,
            format!("not json\n{usage}\n{usage}\n").as_bytes(),
            &mut events,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(state.book.today_totals.total(), 12);
    }

    #[test]
    fn picks_up_appends_through_poll_file() {
        let fixture = fs_fixture!({
            "logs/unified.jsonl": format!("{}\n", inference_line("session-a", 10, 0, 2, 0)),
        });
        let path = fixture.path("logs/unified.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let size = fs::metadata(&path).unwrap().len();
        state.poll_file(&path, size, &mut events);
        assert_eq!(events.len(), 1);

        use std::io::Write as _;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", inference_line("session-b", 20, 0, 4, 1)).unwrap();
        let size = fs::metadata(&path).unwrap().len();
        state.poll_file(&path, size, &mut events);

        assert_eq!(events.len(), 2);
        assert_eq!(events[1].session_id.as_ref(), "session-b");
        assert_eq!(events[1].usage.output_tokens, 5);
        assert_eq!(state.book.today_totals.total(), 12 + 25);
    }

    #[test]
    fn fires_a_token_alert_once_when_todays_totals_cross() {
        let mut state = live_state();
        state.alert_state = AlertState::new(AlertThresholds {
            cost: None,
            tokens: Some(10),
        });
        let mut events = Vec::new();
        state.feed_bytes(
            Path::new("/tmp/grok/logs/unified.jsonl"),
            format!("{}\n", inference_line("session-a", 10, 0, 2, 0)).as_bytes(),
            &mut events,
        );

        let fired = state.check_alerts();
        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].metric,
            turbotokens_adapter_common::live::AlertMetric::Tokens
        );
        assert_eq!(fired[0].value, 12.0);
        assert!(state.check_alerts().is_empty());
    }
}
