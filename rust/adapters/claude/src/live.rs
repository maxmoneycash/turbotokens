use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Duration,
};

use serde_json::json;

use turbotokens_adapter_common::{
    live::{
        Alert, AlertState, AlertThresholds, Burn, Dashboard, DashboardView, LiveBook, LiveEvent,
        LiveMetrics, LiveOutput, MetricsServer, TokenTotals, detect_output, map_stream_result,
        render_prometheus, write_human_line, write_json_line,
    },
    read_files_parallel,
};
use turbotokens_core::{Result, json_float};

use crate::{
    cli::{LiveArgs, SharedArgs},
    daily::DailyLoadedEntry,
    fast::FxHashMap,
    paths::usage_files,
    watch::{WatchIndex, WatchOutcome},
};

const WEBHOOK_TIMEOUT_SECONDS: u64 = 5;

/// Streams Claude Code token usage as it is appended to the JSONL logs,
/// either as NDJSON events (`--json`), one human-readable line per event
/// (piped stdout), or an in-place terminal dashboard (TTY). Optionally fires
/// threshold alerts (stderr/banner/webhook) and serves a Prometheus metrics
/// endpoint (`--serve`).
pub fn run_live(args: &LiveArgs) -> Result<()> {
    let shared = &args.shared;
    let paths = crate::paths::claude_paths()?;
    let mut state = LiveState::new(
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

    // Seed from the existing logs: parallel reads feed the same per-chunk
    // handler the poller uses for appended bytes.
    let files = usage_files(&paths, None);
    let contents = read_files_parallel(&files, shared.single_thread, |file| {
        fs::read(file).unwrap_or_default()
    });
    let mut events = Vec::new();
    for (file, bytes) in files.iter().zip(contents) {
        state.feed_bytes(file, &bytes, &mut events);
    }
    state.live = true;

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut dashboard = Dashboard::default();
    let startup = emit_startup(
        output_mode,
        shared,
        &mut state,
        &paths,
        args.interval_ms,
        &events,
        files.len(),
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
        let files = usage_files(&paths, None);
        events.clear();
        for file in &files {
            let Ok(metadata) = fs::metadata(file) else {
                continue;
            };
            state.poll_file(file, metadata.len(), &mut events);
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
            &paths,
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
    state: &mut LiveState,
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

fn update_metrics(server: &Option<MetricsServer>, state: &mut LiveState) {
    if let Some(server) = server {
        server.update(render_prometheus(&state.live_metrics()));
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_startup(
    output_mode: LiveOutput,
    shared: &SharedArgs,
    state: &mut LiveState,
    paths: &[PathBuf],
    interval_ms: u64,
    events: &[LiveEvent],
    files: usize,
    dashboard: &mut Dashboard,
    out: &mut impl Write,
) -> io::Result<()> {
    match output_mode {
        LiveOutput::Json => {
            write_json_line(out, &state.snapshot_json(files))?;
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
            dashboard.render(shared, &state.dashboard_view(paths, interval_ms), out)?;
        }
    }
    out.flush()
}

#[allow(clippy::too_many_arguments)]
fn emit_tick(
    output_mode: LiveOutput,
    shared: &SharedArgs,
    state: &mut LiveState,
    paths: &[PathBuf],
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
                dashboard.render(shared, &state.dashboard_view(paths, interval_ms), out)?;
            }
        }
    }
    out.flush()
}

struct LiveState {
    index: WatchIndex,
    book: LiveBook,
    burn: Burn,
    alert_state: AlertState,
    /// Latest alert banner shown as a dashboard row; sticky once fired.
    alert_banner: Option<String>,
    /// False while seeding from existing logs: historical entries must not
    /// count toward the live burn rate.
    live: bool,
}

impl LiveState {
    fn new(shared: &SharedArgs, thresholds: AlertThresholds) -> Self {
        let index = WatchIndex::new(shared);
        let today = index.today();
        Self {
            index,
            book: LiveBook::new(today),
            burn: Burn::default(),
            alert_state: AlertState::new(thresholds),
            alert_banner: None,
            live: false,
        }
    }

    /// Feeds raw bytes for one file through the incremental scanner, emitting
    /// an event for every newly accepted entry.
    fn feed_bytes(&mut self, path: &Path, bytes: &[u8], events: &mut Vec<LiveEvent>) {
        let mut outcomes = Vec::new();
        self.index
            .feed_bytes(path, bytes, &mut |outcome| outcomes.push(outcome));
        for outcome in outcomes {
            self.accept_outcome(outcome, events);
        }
    }

    fn poll_file(&mut self, path: &Path, size: u64, events: &mut Vec<LiveEvent>) {
        let mut outcomes = Vec::new();
        self.index
            .poll_file(path, size, &mut |outcome| outcomes.push(outcome));
        for outcome in outcomes {
            self.accept_outcome(outcome, events);
        }
    }

    fn accept_outcome(&mut self, outcome: WatchOutcome, events: &mut Vec<LiveEvent>) {
        match outcome {
            WatchOutcome::Added { index, session_id } => {
                let event = event_for_entry(&self.index.deduped[index], &session_id);
                self.book.add_contribution(&event);
                if self.live {
                    self.burn.push(event.total_tokens());
                }
                self.book.push_recent(event.clone());
                events.push(event);
            }
            WatchOutcome::Replaced {
                index,
                previous,
                session_id,
            } => {
                let previous_event = event_for_entry(&previous, &session_id);
                self.book.subtract_contribution(&previous_event);
                let event = event_for_entry(&self.index.deduped[index], &session_id);
                self.book.add_contribution(&event);
                if self.live {
                    self.burn.push(
                        event
                            .total_tokens()
                            .saturating_sub(previous_event.total_tokens()),
                    );
                }
            }
        }
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
            files_watched: self.index.cursors.len() as u64,
        }
    }

    fn dashboard_view(&mut self, paths: &[PathBuf], interval_ms: u64) -> DashboardView<'_> {
        let burn_sparkline = self.burn.sparkline(12);
        let burn_rate = self.burn.rate();
        DashboardView {
            dirs: paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            interval_ms,
            files_watched: self.index.cursors.len(),
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
        let today = self.index.today();
        if today == self.book.today {
            return;
        }
        let mut today_totals = TokenTotals::default();
        let mut model_totals = FxHashMap::<String, TokenTotals>::default();
        for entry in &self.index.deduped {
            if entry.date.as_ref() != today {
                continue;
            }
            today_totals.add(entry.usage, entry.cost);
            if let Some(model) = &entry.model {
                model_totals
                    .entry(model.clone())
                    .or_default()
                    .add(entry.usage, entry.cost);
            }
        }
        self.book.today = today;
        self.book.today_totals = today_totals;
        self.book.model_totals = model_totals;
    }

    fn snapshot_json(&self, files: usize) -> serde_json::Value {
        json!({
            "type": "snapshot",
            "agent": "claude",
            "date": self.book.today,
            "files": files,
            "inputTokens": self.book.today_totals.input_tokens,
            "outputTokens": self.book.today_totals.output_tokens,
            "cacheCreationTokens": self.book.today_totals.cache_creation_tokens,
            "cacheReadTokens": self.book.today_totals.cache_read_tokens,
            "totalTokens": self.book.today_totals.total(),
            "cost": json_float(self.book.today_totals.cost),
        })
    }
}

fn event_for_entry(entry: &DailyLoadedEntry, session_id: &Arc<str>) -> LiveEvent {
    LiveEvent {
        timestamp_ms: entry.timestamp_ms,
        date: entry.date.to_string(),
        project: Arc::clone(&entry.project),
        session_id: Arc::clone(session_id),
        model: entry.model.clone(),
        usage: entry.usage,
        cost: entry.cost,
        agent: "claude",
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use turbotokens_test_support::fs_fixture;

    use super::*;
    use crate::cli::CostMode;

    fn live_state() -> LiveState {
        let mut state = LiveState::new(
            &SharedArgs {
                mode: CostMode::Display,
                timezone: Some("UTC".to_string()),
                ..SharedArgs::default()
            },
            AlertThresholds::default(),
        );
        // Match the fixed timestamp usage_line stamps on its entries.
        state.book.today = "2026-07-27".to_string();
        state
    }

    fn usage_line(message_id: &str, output_tokens: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-07-27T18:00:00.000Z","version":"1.2.3","sessionId":"sess-1","message":{{"id":"{message_id}","model":"claude-sonnet-4","usage":{{"input_tokens":100,"output_tokens":{output_tokens},"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}},"requestId":"req-{message_id}","costUSD":0.0123}}"#
        )
    }

    #[test]
    fn emits_events_for_appended_complete_lines() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let bytes = format!("{}\n{}\n", usage_line("msg-1", 20), usage_line("msg-2", 30));

        state.feed_bytes(path, bytes.as_bytes(), &mut events);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].session_id.as_ref(), "sess-1");
        assert_eq!(events[0].project.as_ref(), "proj-a");
        assert_eq!(events[0].model.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(events[0].agent, "claude");
        assert_eq!(events[0].usage.output_tokens, 20);
        assert_eq!(events[0].total_tokens(), 135);
        assert_eq!(events[0].cost, 0.0123);
        assert_eq!(state.book.today_totals.total(), 135 + 145);
        assert!((state.book.today_totals.cost - 0.0246).abs() < 1e-9);
    }

    #[test]
    fn buffers_a_partial_line_until_its_newline_arrives() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let line = usage_line("msg-1", 20);
        let split = line.len() / 2;

        state.feed_bytes(path, line.as_bytes()[..split].as_ref(), &mut events);
        assert!(events.is_empty());

        state.feed_bytes(
            path,
            format!("{}\n", &line[split..]).as_bytes(),
            &mut events,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].total_tokens(), 135);

        // The cursor sits on the newline boundary with nothing carried over.
        let cursor = state.index.cursors.get(path).unwrap();
        assert_eq!(cursor.offset as usize, line.len() + 1);
        assert!(cursor.tail.is_empty());
    }

    #[test]
    fn buffers_and_counts_whitespace_formatted_usage() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let line = usage_line("msg-1", 20).replace("\":", "\" \t: \t");
        let split = line.find("\"usage\"").unwrap() + "\"usage\" ".len();

        state.feed_bytes(path, &line.as_bytes()[..split], &mut events);
        assert!(events.is_empty());
        state.feed_bytes(
            path,
            format!("{}\r\n", &line[split..]).as_bytes(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].total_tokens(), 135);
        assert_eq!(state.book.today_totals.total(), 135);
        assert!(state.index.cursors.get(path).unwrap().tail.is_empty());
    }

    #[test]
    fn dedupes_replayed_lines_across_feeds() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let bytes = format!("{}\n", usage_line("msg-1", 20));

        state.feed_bytes(path, bytes.as_bytes(), &mut events);
        state.feed_bytes(path, bytes.as_bytes(), &mut events);

        assert_eq!(events.len(), 1);
        assert_eq!(state.book.today_totals.total(), 135);
    }

    #[test]
    fn adjusts_totals_when_a_more_complete_replay_replaces_an_entry() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();

        state.feed_bytes(
            path,
            format!("{}\n", usage_line("msg-1", 20)).as_bytes(),
            &mut events,
        );
        state.feed_bytes(
            path,
            format!("{}\n", usage_line("msg-1", 250)).as_bytes(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(state.book.today_totals.total(), 100 + 250 + 10 + 5);
    }

    #[test]
    fn rescans_a_shrunk_file_from_offset_zero() {
        let fixture = fs_fixture!({
            "projects/proj-a/sess-1.jsonl": format!("{}\n", usage_line("msg-1", 20)),
        });
        let path = fixture.path("projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        let size = fs::metadata(&path).unwrap().len();
        state.poll_file(&path, size, &mut events);
        assert_eq!(events.len(), 1);

        // Rewritten shorter, same message id: dedup keeps the totals steady.
        std::fs::write(&path, format!("{}\n", usage_line("msg-1", 20))).unwrap();
        let shrunk = fs::metadata(&path).unwrap().len();
        state.poll_file(&path, shrunk, &mut events);

        assert_eq!(events.len(), 1);
        assert_eq!(state.book.today_totals.total(), 135);

        // Grown afterwards: only the appended line produces a new event.
        let mut grown = format!("{}\n", usage_line("msg-1", 20));
        grown.push_str(&format!("{}\n", usage_line("msg-2", 30)));
        std::fs::write(&path, &grown).unwrap();
        let size = fs::metadata(&path).unwrap().len();
        state.poll_file(&path, size, &mut events);

        assert_eq!(events.len(), 2);
        assert_eq!(events[1].total_tokens(), 145);
    }

    #[test]
    fn rolls_today_totals_over_with_the_configured_timezone() {
        let mut state = live_state();
        state.book.today = "1999-01-01".to_string();
        let mut events = Vec::new();
        state.feed_bytes(
            Path::new("/tmp/projects/proj-a/sess-1.jsonl"),
            format!("{}\n", usage_line("msg-1", 20)).as_bytes(),
            &mut events,
        );
        // The seeded entry is dated 2026-07-27, not the stale "today".
        assert_eq!(state.book.today_totals.total(), 0);

        state.refresh_today();

        assert_ne!(state.book.today, "1999-01-01");
        let expected = if state.book.today == "2026-07-27" {
            135
        } else {
            0
        };
        assert_eq!(state.book.today_totals.total(), expected);
    }

    #[test]
    fn fires_a_token_alert_once_when_todays_totals_cross() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        state.alert_state = AlertState::new(AlertThresholds {
            cost: None,
            tokens: Some(100),
        });
        let mut events = Vec::new();

        assert!(state.check_alerts().is_empty());
        state.feed_bytes(
            path,
            format!("{}\n", usage_line("msg-1", 20)).as_bytes(),
            &mut events,
        );

        let fired = state.check_alerts();
        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].to_json(),
            serde_json::json!({
                "type": "alert",
                "metric": "tokens",
                "threshold": 100,
                "value": 135,
                "date": "2026-07-27",
            })
        );

        // Staying above the threshold does not re-fire.
        state.feed_bytes(
            path,
            format!("{}\n", usage_line("msg-2", 30)).as_bytes(),
            &mut events,
        );
        assert!(state.check_alerts().is_empty());
    }

    #[test]
    fn fires_a_cost_alert_against_todays_cost() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        state.alert_state = AlertState::new(AlertThresholds {
            cost: Some(0.01),
            tokens: None,
        });
        let mut events = Vec::new();

        state.feed_bytes(
            path,
            format!("{}\n", usage_line("msg-1", 20)).as_bytes(),
            &mut events,
        );

        let fired = state.check_alerts();
        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].metric,
            turbotokens_adapter_common::live::AlertMetric::Cost
        );
        assert!((fired[0].value - 0.0123).abs() < 1e-9);
        assert!(state.check_alerts().is_empty());
    }

    #[test]
    fn builds_prometheus_metrics_from_the_resident_state() {
        let path = Path::new("/tmp/projects/proj-a/sess-1.jsonl");
        let mut state = live_state();
        let mut events = Vec::new();
        state.feed_bytes(
            path,
            format!("{}\n", usage_line("msg-1", 20)).as_bytes(),
            &mut events,
        );

        let metrics = state.live_metrics();

        assert_eq!(metrics.input_tokens, 100);
        assert_eq!(metrics.output_tokens, 20);
        assert_eq!(metrics.cache_creation_tokens, 10);
        assert_eq!(metrics.cache_read_tokens, 5);
        assert!((metrics.cost_usd - 0.0123).abs() < 1e-9);
        assert_eq!(metrics.files_watched, 1);
        assert_eq!(
            metrics.model_tokens,
            vec![("claude-sonnet-4".to_string(), 135)]
        );
    }
}
