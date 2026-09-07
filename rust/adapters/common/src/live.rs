//! Agent-agnostic machinery for `turbotokens live`: token/cost bookkeeping, the
//! terminal dashboard, edge-triggered threshold alerts, and a Prometheus text
//! metrics endpoint. Each agent adapter owns its log parsing and feeds this
//! module plain [`LiveEvent`]s.

use std::{
    collections::VecDeque,
    io::{self, IsTerminal, Write},
    net::TcpListener,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use turbotokens_core::{
    Color, TimestampMs, TokenUsageRaw, cli::SharedArgs, color, fast::FxHashMap, format_currency,
    format_number, format_rfc3339_millis, json_float, terminal_width, truncate_to_width, utc_now,
};

/// Window for the trailing burn rate and for counting active sessions.
pub const ACTIVITY_WINDOW: Duration = Duration::from_secs(300);
const MAX_MODELS: usize = 5;
const MAX_SESSIONS: usize = 8;
const MAX_RECENT_EVENTS: usize = 10;
const DASHBOARD_CLOCK_TICK: Duration = Duration::from_secs(1);

/// One accepted usage line, ready to serialize or render.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveEvent {
    pub timestamp_ms: i64,
    pub date: String,
    pub project: Arc<str>,
    pub session_id: Arc<str>,
    pub model: Option<String>,
    pub usage: TokenUsageRaw,
    pub cost: f64,
    /// Live source: `claude`, `codex`, or `grok`.
    pub agent: &'static str,
}

impl LiveEvent {
    pub fn total_tokens(&self) -> u64 {
        total_tokens(self.usage)
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "type": "usage",
            "timestamp": format_rfc3339_millis(TimestampMs::from_millis(self.timestamp_ms)),
            "agent": self.agent,
            "project": self.project.as_ref(),
            "sessionId": self.session_id.as_ref(),
            "model": self.model,
            "inputTokens": self.usage.input_tokens,
            "outputTokens": self.usage.output_tokens,
            "cacheCreationTokens": self.usage.cache_creation_token_count(),
            "cacheReadTokens": self.usage.cache_read_input_tokens,
            "totalTokens": self.total_tokens(),
            "cost": json_float(self.cost),
        })
    }
}

pub fn total_tokens(usage: TokenUsageRaw) -> u64 {
    usage.input_tokens
        + usage.output_tokens
        + usage.cache_creation_token_count()
        + usage.cache_read_input_tokens
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost: f64,
}

impl TokenTotals {
    pub fn add(&mut self, usage: TokenUsageRaw, cost: f64) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.cache_creation_tokens += usage.cache_creation_token_count();
        self.cache_read_tokens += usage.cache_read_input_tokens;
        self.cost += cost;
    }

    pub fn subtract(&mut self, usage: TokenUsageRaw, cost: f64) {
        self.input_tokens -= usage.input_tokens;
        self.output_tokens -= usage.output_tokens;
        self.cache_creation_tokens -= usage.cache_creation_token_count();
        self.cache_read_tokens -= usage.cache_read_input_tokens;
        self.cost -= cost;
    }

    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }
}

#[derive(Debug, Default)]
pub struct SessionStats {
    pub project: Arc<str>,
    pub models: Vec<String>,
    pub totals: TokenTotals,
    pub last_activity_ms: i64,
}

/// Resident per-day/per-model/per-session totals every live loop maintains.
#[derive(Debug, Default)]
pub struct LiveBook {
    pub today: String,
    pub today_totals: TokenTotals,
    pub model_totals: FxHashMap<String, TokenTotals>,
    pub sessions: FxHashMap<(Arc<str>, Arc<str>), SessionStats>,
    pub recent: VecDeque<LiveEvent>,
}

impl LiveBook {
    pub fn new(today: String) -> Self {
        Self {
            today,
            ..Self::default()
        }
    }

    pub fn add_contribution(&mut self, event: &LiveEvent) {
        if event.date == self.today {
            self.today_totals.add(event.usage, event.cost);
            if let Some(model) = &event.model {
                self.model_totals
                    .entry(model.clone())
                    .or_default()
                    .add(event.usage, event.cost);
            }
        }
        let session = self
            .sessions
            .entry((Arc::clone(&event.project), Arc::clone(&event.session_id)))
            .or_default();
        session.project = Arc::clone(&event.project);
        session.totals.add(event.usage, event.cost);
        session.last_activity_ms = session.last_activity_ms.max(event.timestamp_ms);
        if let Some(model) = &event.model
            && !session.models.contains(model)
        {
            session.models.push(model.clone());
        }
    }

    pub fn subtract_contribution(&mut self, event: &LiveEvent) {
        if event.date == self.today {
            self.today_totals.subtract(event.usage, event.cost);
            if let Some(model) = &event.model
                && let Some(totals) = self.model_totals.get_mut(model)
            {
                totals.subtract(event.usage, event.cost);
            }
        }
        if let Some(session) = self
            .sessions
            .get_mut(&(Arc::clone(&event.project), Arc::clone(&event.session_id)))
        {
            session.totals.subtract(event.usage, event.cost);
        }
    }

    pub fn push_recent(&mut self, event: LiveEvent) {
        self.recent.push_back(event);
        while self.recent.len() > MAX_RECENT_EVENTS {
            self.recent.pop_front();
        }
    }

    /// Sessions with activity inside the trailing [`ACTIVITY_WINDOW`].
    pub fn sessions_active(&self) -> u64 {
        let now = utc_now().as_millis();
        self.sessions
            .values()
            .filter(|session| {
                now.saturating_sub(session.last_activity_ms) < ACTIVITY_WINDOW.as_millis() as i64
            })
            .count() as u64
    }

    pub fn session_views(&self) -> Vec<SessionView> {
        let mut sessions = self
            .sessions
            .iter()
            .map(|((_, session_id), stats)| SessionView {
                project: stats.project.to_string(),
                session_id: session_id.to_string(),
                models: stats.models.clone(),
                totals: stats.totals.clone(),
                last_activity_ms: stats.last_activity_ms,
            })
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.last_activity_ms));
        sessions
    }
}

/// Trailing five-minute token burn. The caller decides when a contribution is
/// "live" (seeded history must not count toward the rate).
#[derive(Debug, Default)]
pub struct Burn {
    window: VecDeque<(Instant, u64)>,
}

impl Burn {
    pub fn push(&mut self, tokens: u64) {
        if tokens > 0 {
            self.window.push_back((Instant::now(), tokens));
        }
    }

    /// Tokens per minute over the trailing five-minute window.
    pub fn rate(&mut self) -> f64 {
        self.prune();
        let tokens = self.window.iter().map(|(_, tokens)| tokens).sum::<u64>();
        let elapsed = self
            .window
            .front()
            .map(|(when, _)| when.elapsed().as_secs())
            .unwrap_or(0)
            .max(60);
        tokens as f64 / (elapsed as f64 / 60.0)
    }

    /// The trailing five-minute window as a sparkline: `buckets` equal time
    /// slots, oldest first, token sums normalized against the fullest bucket.
    pub fn sparkline(&mut self, buckets: usize) -> String {
        const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        self.prune();
        if buckets == 0 {
            return String::new();
        }
        let now = Instant::now();
        let window = ACTIVITY_WINDOW.as_secs_f64();
        let mut sums = vec![0u64; buckets];
        for (when, tokens) in &self.window {
            let age = now.duration_since(*when).as_secs_f64().min(window);
            let slot = (((window - age) / window) * buckets as f64) as usize;
            sums[slot.min(buckets - 1)] += tokens;
        }
        let max = sums.iter().copied().max().unwrap_or(0).max(1);
        sums.iter()
            .map(|sum| LEVELS[(sum * 7 / max) as usize])
            .collect()
    }

    fn prune(&mut self) {
        let cutoff = Instant::now() - ACTIVITY_WINDOW;
        while self.window.front().is_some_and(|(when, _)| *when < cutoff) {
            self.window.pop_front();
        }
    }
}

/// Display name for a model id: drops the `claude-` vendor prefix and a
/// trailing `-YYYYMMDD` date stamp, keeps everything else.
pub fn short_model(model: &str) -> &str {
    let name = model.strip_prefix("claude-").unwrap_or(model);
    match name.rsplit_once('-') {
        Some((base, suffix))
            if suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            base
        }
        _ => name,
    }
}

/// Compact burn-rate label: 842 → "842", 18,400 → "18.4k", 2,300,000 → "2.3M".
pub fn format_rate(rate: f64) -> String {
    if rate < 1_000.0 {
        format!("{rate:.0}")
    } else if rate < 1_000_000.0 {
        format!("{:.1}k", rate / 1_000.0)
    } else {
        format!("{:.1}M", rate / 1_000_000.0)
    }
}

/// Read position of one watched file: `offset` bytes were consumed as complete
/// lines, `tail` holds the bytes after the last newline seen so far.
#[derive(Debug, Default)]
pub struct ByteCursor {
    pub offset: u64,
    pub tail: Vec<u8>,
}

impl ByteCursor {
    pub fn position(&self) -> u64 {
        self.offset + self.tail.len() as u64
    }

    /// Appends `bytes` and calls `line` for each newline-terminated line; the
    /// unterminated tail is carried into the next feed.
    pub fn feed(&mut self, bytes: &[u8], mut line: impl FnMut(&[u8])) {
        self.tail.extend_from_slice(bytes);
        let mut consumed = 0;
        while let Some(newline) = memchr::memchr(b'\n', &self.tail[consumed..]) {
            line(&self.tail[consumed..consumed + newline]);
            consumed += newline + 1;
        }
        self.tail.drain(..consumed);
        self.offset += consumed as u64;
    }
}

/// Reads the bytes appended to `path` after `position`.
pub fn read_appended(path: &std::path::Path, position: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(position)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum LiveOutput {
    Json,
    Human,
    Dashboard,
}

/// NDJSON events with `--json`, an in-place dashboard on a TTY, and one
/// human-readable line per event on piped stdout.
pub fn detect_output(json: bool) -> LiveOutput {
    if json {
        LiveOutput::Json
    } else if io::stdout().is_terminal() {
        LiveOutput::Dashboard
    } else {
        LiveOutput::Human
    }
}

/// A broken pipe is the natural end of a piped stream (`turbotokens stream |
/// head`); every other I/O error is real.
pub fn map_stream_result(result: io::Result<()>) -> turbotokens_core::Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn write_json_line(out: &mut impl Write, value: &serde_json::Value) -> io::Result<()> {
    writeln!(out, "{value}")?;
    out.flush()
}

pub fn write_human_line(out: &mut impl Write, event: &LiveEvent) -> io::Result<()> {
    writeln!(
        out,
        "{}  {}  {} tok  ${:.4}  {}/{}",
        format_rfc3339_millis(TimestampMs::from_millis(event.timestamp_ms)),
        event.model.as_deref().unwrap_or("unknown"),
        format_number(event.total_tokens()),
        event.cost,
        event.project,
        event.session_id,
    )?;
    out.flush()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AlertThresholds {
    pub cost: Option<f64>,
    pub tokens: Option<u64>,
}

impl AlertThresholds {
    pub fn is_armed(&self) -> bool {
        self.cost.is_some() || self.tokens.is_some()
    }
}

/// Edge-triggered threshold alerts: each threshold fires once per process when
/// today's totals cross it, and re-arms only on restart.
#[derive(Debug, Default)]
pub struct AlertState {
    thresholds: AlertThresholds,
    cost_fired: bool,
    tokens_fired: bool,
}

impl AlertState {
    pub fn new(thresholds: AlertThresholds) -> Self {
        Self {
            thresholds,
            cost_fired: false,
            tokens_fired: false,
        }
    }

    pub fn check(&mut self, date: &str, cost: f64, tokens: u64) -> Vec<Alert> {
        let mut fired = Vec::new();
        if !self.cost_fired
            && let Some(threshold) = self.thresholds.cost
            && cost >= threshold
        {
            self.cost_fired = true;
            fired.push(Alert {
                metric: AlertMetric::Cost,
                threshold,
                value: cost,
                date: date.to_string(),
            });
        }
        if !self.tokens_fired
            && let Some(threshold) = self.thresholds.tokens
            && tokens >= threshold
        {
            self.tokens_fired = true;
            fired.push(Alert {
                metric: AlertMetric::Tokens,
                threshold: threshold as f64,
                value: tokens as f64,
                date: date.to_string(),
            });
        }
        fired
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AlertMetric {
    Cost,
    Tokens,
}

impl AlertMetric {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertMetric::Cost => "cost",
            AlertMetric::Tokens => "tokens",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub metric: AlertMetric,
    pub threshold: f64,
    pub value: f64,
    pub date: String,
}

impl Alert {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "type": "alert",
            "metric": self.metric.as_str(),
            "threshold": json_float(self.threshold),
            "value": json_float(self.value),
            "date": self.date,
        })
    }

    pub fn banner(&self) -> String {
        match self.metric {
            AlertMetric::Cost => format!(
                "⚠ ALERT: today's cost {} crossed the {} threshold ({})",
                format_currency(self.value),
                format_currency(self.threshold),
                self.date,
            ),
            AlertMetric::Tokens => format!(
                "⚠ ALERT: today's tokens {} crossed the {} threshold ({})",
                format_number(self.value as u64),
                format_number(self.threshold as u64),
                self.date,
            ),
        }
    }
}

/// Point-in-time values served at the Prometheus endpoint; every scrape
/// reflects the resident state at that moment.
#[derive(Debug, Default)]
pub struct LiveMetrics {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub tokens_per_minute: f64,
    pub model_tokens: Vec<(String, u64)>,
    pub sessions_active: u64,
    pub files_watched: u64,
}

pub fn render_prometheus(metrics: &LiveMetrics) -> String {
    let mut text = String::new();
    text.push_str("# HELP turbotokens_tokens_total Tokens used today by kind.\n");
    text.push_str("# TYPE turbotokens_tokens_total gauge\n");
    for (kind, value) in [
        ("input", metrics.input_tokens),
        ("output", metrics.output_tokens),
        ("cache_creation", metrics.cache_creation_tokens),
        ("cache_read", metrics.cache_read_tokens),
    ] {
        text.push_str(&format!(
            "turbotokens_tokens_total{{kind=\"{kind}\"}} {value}\n"
        ));
    }
    text.push_str("# HELP turbotokens_cost_usd_total Cost of today's usage in USD.\n");
    text.push_str("# TYPE turbotokens_cost_usd_total gauge\n");
    text.push_str(&format!(
        "turbotokens_cost_usd_total {}\n",
        metrics.cost_usd
    ));
    text.push_str(
        "# HELP turbotokens_tokens_per_minute Token burn rate over the trailing 5 minutes.\n",
    );
    text.push_str("# TYPE turbotokens_tokens_per_minute gauge\n");
    text.push_str(&format!(
        "turbotokens_tokens_per_minute {}\n",
        metrics.tokens_per_minute
    ));
    text.push_str("# HELP turbotokens_model_tokens_total Tokens used today by model.\n");
    text.push_str("# TYPE turbotokens_model_tokens_total gauge\n");
    for (model, tokens) in &metrics.model_tokens {
        text.push_str(&format!(
            "turbotokens_model_tokens_total{{model=\"{}\"}} {tokens}\n",
            escape_label_value(model),
        ));
    }
    text.push_str(
        "# HELP turbotokens_sessions_active Sessions with activity in the last 5 minutes.\n",
    );
    text.push_str("# TYPE turbotokens_sessions_active gauge\n");
    text.push_str(&format!(
        "turbotokens_sessions_active {}\n",
        metrics.sessions_active
    ));
    text.push_str("# HELP turbotokens_files_watched JSONL log files currently tracked.\n");
    text.push_str("# TYPE turbotokens_files_watched gauge\n");
    text.push_str(&format!(
        "turbotokens_files_watched {}\n",
        metrics.files_watched
    ));
    text
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Minimal HTTP/1.1 endpoint serving the latest Prometheus snapshot: one
/// thread, one 200 response per connection, then close.
pub struct MetricsServer {
    body: Arc<Mutex<String>>,
}

impl MetricsServer {
    pub fn start(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let body = Arc::new(Mutex::new(String::new()));
        let shared = Arc::clone(&body);
        thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut stream) = connection else {
                    continue;
                };
                let payload = lock_body(&shared).clone();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len(),
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Ok(Self { body })
    }

    pub fn update(&self, body: String) {
        *lock_body(&self.body) = body;
    }
}

fn lock_body(body: &Arc<Mutex<String>>) -> MutexGuard<'_, String> {
    body.lock().unwrap_or_else(|error| error.into_inner())
}

/// Everything the dashboard needs for one render, pre-digested by the adapter.
pub struct DashboardView<'a> {
    pub dirs: String,
    pub interval_ms: u64,
    pub files_watched: usize,
    pub today: &'a str,
    pub today_totals: &'a TokenTotals,
    pub burn_rate: f64,
    pub burn_sparkline: String,
    pub models: Vec<(String, TokenTotals)>,
    pub sessions: Vec<SessionView>,
    pub recent: &'a VecDeque<LiveEvent>,
    pub alert_banner: Option<&'a str>,
}

pub struct SessionView {
    pub project: String,
    pub session_id: String,
    pub models: Vec<String>,
    pub totals: TokenTotals,
    pub last_activity_ms: i64,
}

#[derive(Default)]
pub struct Dashboard {
    rendered_lines: usize,
    last_render: Option<Instant>,
}

impl Dashboard {
    /// Redraw at most once per second unless a tick carried new events.
    pub fn should_render(&self, has_events: bool) -> bool {
        has_events
            || self
                .last_render
                .is_none_or(|when| when.elapsed() >= DASHBOARD_CLOCK_TICK)
    }

    pub fn render(
        &mut self,
        shared: &SharedArgs,
        view: &DashboardView,
        out: &mut impl Write,
    ) -> io::Result<()> {
        let width = terminal_width();
        let mut lines = dashboard_lines(shared, view);
        // Keep the line count constant so the in-place redraw never leaves
        // stale rows behind.
        lines.resize(self.rendered_lines.max(lines.len()), String::new());
        if self.rendered_lines > 0 {
            write!(out, "\x1b[{}A", self.rendered_lines)?;
        }
        for line in &lines {
            write!(out, "\x1b[2K{}\r\n", truncate_to_width(line, width))?;
        }
        self.rendered_lines = lines.len();
        self.last_render = Some(Instant::now());
        out.flush()
    }
}

fn dashboard_lines(shared: &SharedArgs, view: &DashboardView) -> Vec<String> {
    let section = |title: &str| color(shared, title, Color::Blue);
    let mut lines = vec![
        color(
            shared,
            format!(
                "turbotokens live — {} files · {} · {}ms",
                view.files_watched, view.dirs, view.interval_ms,
            ),
            Color::Blue,
        ),
        String::new(),
        format!(
            "Today ({})   {} tok   {}",
            view.today,
            format_number(view.today_totals.total()),
            format_currency(view.today_totals.cost),
        ),
        format!(
            "{}  {} tok/min (5m)",
            color(shared, &view.burn_sparkline, Color::Green),
            format_rate(view.burn_rate),
        ),
    ];
    if let Some(banner) = view.alert_banner {
        lines.push(color(shared, banner, Color::Red));
    }
    lines.push(String::new());
    lines.push(section("Top models:"));
    let mut models = view
        .models
        .iter()
        .map(|(model, totals)| (model.clone(), totals.clone()))
        .collect::<Vec<_>>();
    models.sort_by_key(|entry| std::cmp::Reverse(entry.1.total()));
    let listed = models.len().min(MAX_MODELS);
    let listed_total = models[..listed]
        .iter()
        .map(|(_, totals)| totals.total())
        .sum::<u64>()
        .max(1);
    for (model, totals) in models.iter().take(listed) {
        let share = totals.total() as f64 / listed_total as f64;
        let filled = ((share * 10.0) as usize).min(10);
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
        lines.push(format!(
            "  {:<10}  {:>10} tok  {:>8}  {}  {:.0}%",
            truncate_to_width(short_model(model), 10),
            format_number(totals.total()),
            format_currency(totals.cost),
            bar,
            share * 100.0,
        ));
    }
    pad_lines(&mut lines, MAX_MODELS.saturating_sub(listed));
    lines.push(String::new());
    lines.push(section("Active sessions:"));
    let now = utc_now().as_millis();
    for session in view.sessions.iter().take(MAX_SESSIONS) {
        let models = session
            .models
            .iter()
            .map(|model| short_model(model))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "  {:<8} {:<8} {:<18} {:>10} tok  {:>8}  {}",
            truncate_to_width(&session.project, 8),
            truncate_to_width(&session.session_id, 8),
            truncate_to_width(&models, 18),
            format_number(session.totals.total()),
            format_currency(session.totals.cost),
            color(
                shared,
                format!("{:>7}", ago(now.saturating_sub(session.last_activity_ms))),
                Color::Grey,
            ),
        ));
    }
    pad_lines(
        &mut lines,
        MAX_SESSIONS.saturating_sub(view.sessions.len().min(MAX_SESSIONS)),
    );
    lines.push(String::new());
    lines.push(section("Recent events:"));
    for event in view.recent.iter().rev().take(MAX_RECENT_EVENTS) {
        let timestamp = format_rfc3339_millis(TimestampMs::from_millis(event.timestamp_ms));
        lines.push(format!(
            "  {}  {}  {:>10} tok  {}",
            color(
                shared,
                timestamp.get(11..19).unwrap_or(&timestamp),
                Color::Grey
            ),
            color(
                shared,
                format!(
                    "{:<12}",
                    truncate_to_width(short_model(event.model.as_deref().unwrap_or("unknown")), 12,)
                ),
                Color::Yellow,
            ),
            format_number(event.total_tokens()),
            color(shared, format!("${:.4}", event.cost), Color::Green),
        ));
    }
    pad_lines(
        &mut lines,
        MAX_RECENT_EVENTS.saturating_sub(view.recent.len().min(MAX_RECENT_EVENTS)),
    );
    lines
}

fn pad_lines(lines: &mut Vec<String>, count: usize) {
    lines.resize(lines.len() + count, String::new());
}

pub fn ago(elapsed_ms: i64) -> String {
    let seconds = elapsed_ms / 1000;
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn fires_each_threshold_once_when_crossed() {
        let mut alerts = AlertState::new(AlertThresholds {
            cost: Some(1.0),
            tokens: Some(100),
        });

        assert!(alerts.check("2026-07-28", 0.5, 50).is_empty());
        let fired = alerts.check("2026-07-28", 1.5, 150);
        assert_eq!(fired.len(), 2);
        assert_eq!(fired[0].metric, AlertMetric::Cost);
        assert_eq!(fired[1].metric, AlertMetric::Tokens);
        // Edge-triggered: staying above the threshold fires nothing more.
        assert!(alerts.check("2026-07-28", 2.0, 200).is_empty());
    }

    #[test]
    fn usage_json_includes_token_counts_and_total() {
        let event = LiveEvent {
            timestamp_ms: 1_785_175_200_000,
            date: "2026-07-27".to_string(),
            project: Arc::from("webapp"),
            session_id: Arc::from("sess-1"),
            model: Some("claude-sonnet-4".to_string()),
            usage: TokenUsageRaw {
                input_tokens: 90,
                output_tokens: 50,
                cache_creation_input_tokens: 10,
                cache_read_input_tokens: 5,
                speed: None,
                cache_creation: None,
            },
            cost: 0.0123,
            agent: "claude",
        };

        assert_eq!(
            event.to_json(),
            json!({
                "type": "usage",
                "timestamp": "2026-07-27T18:00:00.000Z",
                "agent": "claude",
                "project": "webapp",
                "sessionId": "sess-1",
                "model": "claude-sonnet-4",
                "inputTokens": 90,
                "outputTokens": 50,
                "cacheCreationTokens": 10,
                "cacheReadTokens": 5,
                "totalTokens": 155,
                "cost": 0.0123,
            })
        );
    }

    #[test]
    fn alert_json_matches_the_webhook_payload_shape() {
        let alert = Alert {
            metric: AlertMetric::Tokens,
            threshold: 100.0,
            value: 145.0,
            date: "2026-07-28".to_string(),
        };

        assert_eq!(
            alert.to_json(),
            json!({
                "type": "alert",
                "metric": "tokens",
                "threshold": 100,
                "value": 145,
                "date": "2026-07-28",
            })
        );
    }

    #[test]
    fn renders_prometheus_text_with_all_families() {
        let metrics = LiveMetrics {
            input_tokens: 90,
            output_tokens: 50,
            cache_creation_tokens: 0,
            cache_read_tokens: 10,
            cost_usd: 0.0123,
            tokens_per_minute: 42.5,
            model_tokens: vec![("gpt-\"5\"".to_string(), 150)],
            sessions_active: 2,
            files_watched: 3,
        };

        let text = render_prometheus(&metrics);

        assert!(text.contains("turbotokens_tokens_total{kind=\"input\"} 90\n"));
        assert!(text.contains("turbotokens_tokens_total{kind=\"output\"} 50\n"));
        assert!(text.contains("turbotokens_tokens_total{kind=\"cache_creation\"} 0\n"));
        assert!(text.contains("turbotokens_tokens_total{kind=\"cache_read\"} 10\n"));
        assert!(text.contains("turbotokens_cost_usd_total 0.0123\n"));
        assert!(text.contains("turbotokens_tokens_per_minute 42.5\n"));
        assert!(text.contains("turbotokens_model_tokens_total{model=\"gpt-\\\"5\\\"\"} 150\n"));
        assert!(text.contains("turbotokens_sessions_active 2\n"));
        assert!(text.contains("turbotokens_files_watched 3\n"));
    }

    #[test]
    fn serves_the_latest_snapshot_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let server = MetricsServer::start(&addr.to_string()).unwrap();
        server.update(render_prometheus(&LiveMetrics {
            files_watched: 7,
            ..LiveMetrics::default()
        }));

        let mut response = String::new();
        std::net::TcpStream::connect(addr)
            .unwrap()
            .read_to_string(&mut response)
            .unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("turbotokens_files_watched 7\n"));

        server.update(render_prometheus(&LiveMetrics {
            files_watched: 9,
            ..LiveMetrics::default()
        }));
        let mut response = String::new();
        std::net::TcpStream::connect(addr)
            .unwrap()
            .read_to_string(&mut response)
            .unwrap();
        assert!(response.contains("turbotokens_files_watched 9\n"));
    }

    #[test]
    fn tracks_sessions_active_within_the_activity_window() {
        let mut book = LiveBook::new("2026-07-28".to_string());
        let usage = TokenUsageRaw {
            input_tokens: 1,
            output_tokens: 1,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            speed: None,
            cache_creation: None,
        };
        book.add_contribution(&LiveEvent {
            timestamp_ms: utc_now().as_millis(),
            date: "2026-07-28".to_string(),
            project: Arc::from("proj"),
            session_id: Arc::from("fresh"),
            model: None,
            usage,
            cost: 0.0,
            agent: "claude",
        });
        book.add_contribution(&LiveEvent {
            timestamp_ms: utc_now().as_millis() - ACTIVITY_WINDOW.as_millis() as i64 - 60_000,
            date: "2026-07-28".to_string(),
            project: Arc::from("proj"),
            session_id: Arc::from("stale"),
            model: None,
            usage,
            cost: 0.0,
            agent: "claude",
        });

        assert_eq!(book.sessions_active(), 1);
    }

    #[test]
    fn byte_cursor_buffers_a_partial_line() {
        let mut cursor = ByteCursor::default();
        let mut lines = Vec::new();
        cursor.feed(b"first\nsec", |line| lines.push(line.to_vec()));
        assert_eq!(lines, vec![b"first".to_vec()]);
        cursor.feed(b"ond\n", |line| lines.push(line.to_vec()));
        assert_eq!(lines, vec![b"first".to_vec(), b"second".to_vec()]);
        assert_eq!(cursor.position(), "first\nsecond\n".len() as u64);
        assert!(cursor.tail.is_empty());
    }

    #[test]
    fn token_totals_add_and_subtract() {
        let mut running = TokenTotals::default();
        running.add(
            TokenUsageRaw {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: 2,
                cache_read_input_tokens: 1,
                speed: None,
                cache_creation: None,
            },
            0.5,
        );
        assert_eq!(running.total(), 18);
        running.subtract(
            TokenUsageRaw {
                input_tokens: 4,
                output_tokens: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                speed: None,
                cache_creation: None,
            },
            0.25,
        );
        assert_eq!(
            running,
            TokenTotals {
                input_tokens: 6,
                output_tokens: 4,
                cache_creation_tokens: 2,
                cache_read_tokens: 1,
                cost: 0.25,
            }
        );
        assert_eq!(running.total(), 13);
    }

    #[test]
    fn shortens_model_ids_for_display() {
        assert_eq!(short_model("claude-sonnet-4-20250514"), "sonnet-4");
        assert_eq!(short_model("claude-opus-4-20250514"), "opus-4");
        assert_eq!(short_model("claude-haiku-4-5-20251001"), "haiku-4-5");
        assert_eq!(short_model("claude-sonnet-4"), "sonnet-4");
        assert_eq!(short_model("gpt-5.3-codex"), "gpt-5.3-codex");
        assert_eq!(short_model("gpt-5.3-codex-fast"), "gpt-5.3-codex-fast");
        assert_eq!(short_model("unknown"), "unknown");
    }

    #[test]
    fn formats_burn_rates_compactly() {
        assert_eq!(format_rate(0.0), "0");
        assert_eq!(format_rate(842.4), "842");
        assert_eq!(format_rate(999.4), "999");
        assert_eq!(format_rate(1_000.0), "1.0k");
        assert_eq!(format_rate(18_400.0), "18.4k");
        assert_eq!(format_rate(2_300_000.0), "2.3M");
    }

    #[test]
    fn sparkline_is_all_flat_when_the_window_is_empty() {
        assert_eq!(Burn::default().sparkline(12), "▁▁▁▁▁▁▁▁▁▁▁▁");
    }

    #[test]
    fn sparkline_marks_a_single_spike_with_the_top_block() {
        let burn = &mut Burn::default();
        burn.push(1_000);

        assert_eq!(burn.sparkline(12), "▁▁▁▁▁▁▁▁▁▁▁█");
    }

    #[test]
    fn sparkline_is_uniform_when_buckets_are_even() {
        let now = Instant::now();
        let slot = ACTIVITY_WINDOW.as_secs_f64() / 12.0;
        let mut burn = Burn::default();
        for bucket in 0..12 {
            let age = Duration::from_secs_f64(
                ACTIVITY_WINDOW.as_secs_f64() - (bucket as f64 + 0.5) * slot,
            );
            burn.window.push_back((now - age, 100));
        }

        assert_eq!(burn.sparkline(12), "████████████");
    }

    #[test]
    fn formats_elapsed_activity() {
        assert_eq!(ago(5_000), "5s ago");
        assert_eq!(ago(120_000), "2m ago");
        assert_eq!(ago(7_200_000), "2h ago");
        assert_eq!(ago(172_800_000), "2d ago");
    }
}
