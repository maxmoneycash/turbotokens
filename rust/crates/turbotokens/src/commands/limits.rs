//! `turbotokens limits` — plan-limit / quota tracking for subscription agents.
//!
//! Reads OAuth credentials from the agents' own credential files and queries the
//! same usage endpoints their UIs use. Credentials are never printed, logged, or
//! written anywhere; they only leave the process inside the `Authorization`
//! header of the matching provider request.

use std::{env, fs, path::PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Color, Context as _, MILLIS_PER_MINUTE, Result, TimestampMs,
    cli::{LimitsArgs, LimitsScope, SharedArgs},
    cli_error, color, format_minute, format_remaining_time, format_rfc3339_millis, home,
    parse_ts_timestamp, print_json_or_jq, utc_now, wants_json,
};

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// One plan-limit window (e.g. the 5-hour or weekly window) for an agent.
#[derive(Debug)]
struct WindowLimit {
    /// The window's name in the provider's API response.
    name: &'static str,
    label: String,
    utilization_percent: f64,
    resets_at: Option<TimestampMs>,
}

#[derive(Debug)]
struct AgentLimits {
    agent: &'static str,
    title: &'static str,
    plan: Option<String>,
    windows: Vec<WindowLimit>,
}

enum AgentOutcome {
    Available(AgentLimits),
    Unavailable {
        agent: &'static str,
        title: &'static str,
        reason: String,
    },
}

impl AgentOutcome {
    fn unavailable(agent: &'static str, title: &'static str, reason: impl Into<String>) -> Self {
        Self::Unavailable {
            agent,
            title,
            reason: reason.into(),
        }
    }

    fn agent(&self) -> &'static str {
        match self {
            Self::Available(limits) => limits.agent,
            Self::Unavailable { agent, .. } => agent,
        }
    }
}

pub(super) fn run(args: &LimitsArgs) -> Result<()> {
    let now = utc_now();
    let mut outcomes = Vec::new();
    if matches!(args.scope, LimitsScope::All | LimitsScope::Claude) {
        outcomes.push(claude_limits());
    }
    if matches!(args.scope, LimitsScope::All | LimitsScope::Codex) {
        outcomes.push(codex_limits());
    }

    if wants_json(&args.shared) {
        print_json_or_jq(
            report_json(&outcomes, now),
            args.shared.jq.as_deref(),
            false,
        )?;
    } else {
        print_text_report(&outcomes, &args.shared, now);
    }

    if outcomes
        .iter()
        .all(|outcome| matches!(outcome, AgentOutcome::Unavailable { .. }))
    {
        let agents = outcomes
            .iter()
            .map(|outcome| outcome.agent())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(cli_error(format!(
            "plan limits are not available for any of: {agents} (see above)"
        )));
    }
    Ok(())
}

// --- Claude Code -----------------------------------------------------------

#[derive(Deserialize)]
struct ClaudeCredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeOauthCredentials>,
}

#[derive(Deserialize)]
struct ClaudeOauthCredentials {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
}

/// `GET https://api.anthropic.com/api/oauth/usage` response: per-window
/// utilization percentage (0-100) and an RFC 3339 reset timestamp.
#[derive(Deserialize)]
struct ClaudeUsageResponse {
    five_hour: Option<ClaudeUsageWindow>,
    seven_day: Option<ClaudeUsageWindow>,
    seven_day_opus: Option<ClaudeUsageWindow>,
}

#[derive(Deserialize)]
struct ClaudeUsageWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

fn claude_limits() -> AgentOutcome {
    let Some(token) = claude_access_token() else {
        return AgentOutcome::unavailable(
            "claude",
            "Claude Code",
            "no OAuth credentials found (set CLAUDE_CODE_OAUTH_TOKEN or sign in with Claude Code so .credentials.json exists)",
        );
    };
    let user_agent = format!("turbotokens/{}", env!("TURBOTOKENS_VERSION"));
    let authorization = format!("Bearer {token}");
    let body = match crate::http::fetch_json_with_headers(
        CLAUDE_USAGE_URL,
        &[
            ("Authorization", authorization.as_str()),
            ("anthropic-beta", "oauth-2025-04-20"),
            ("User-Agent", user_agent.as_str()),
            ("Accept", "application/json"),
        ],
    ) {
        Ok(body) => body,
        Err(error) => {
            return AgentOutcome::unavailable(
                "claude",
                "Claude Code",
                fetch_error_reason(CLAUDE_USAGE_URL, &error, "claude"),
            );
        }
    };
    match parse_claude_usage(&body) {
        Ok(limits) => AgentOutcome::Available(limits),
        Err(error) => AgentOutcome::unavailable("claude", "Claude Code", error.to_string()),
    }
}

fn parse_claude_usage(body: &str) -> Result<AgentLimits> {
    let response: ClaudeUsageResponse = serde_json::from_str(body)
        .context("Unexpected response shape from the Claude usage endpoint")?;
    let mut windows = Vec::new();
    for (name, label, window) in [
        ("five_hour", "5-hour window", response.five_hour),
        ("seven_day", "Weekly window", response.seven_day),
        (
            "seven_day_opus",
            "Weekly Opus window",
            response.seven_day_opus,
        ),
    ] {
        let Some(window) = window else {
            continue;
        };
        let Some(utilization) = window.utilization else {
            continue;
        };
        windows.push(WindowLimit {
            name,
            label: label.to_string(),
            utilization_percent: utilization,
            resets_at: window.resets_at.as_deref().and_then(parse_ts_timestamp),
        });
    }
    if windows.is_empty() {
        return Err(cli_error(
            "the Claude usage endpoint response contained no known windows (five_hour / seven_day); the API shape may have changed",
        ));
    }
    Ok(AgentLimits {
        agent: "claude",
        title: "Claude Code",
        plan: None,
        windows,
    })
}

fn claude_access_token() -> Option<String> {
    if let Ok(token) = env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    claude_config_dirs().into_iter().find_map(|dir| {
        let content = fs::read_to_string(dir.join(".credentials.json")).ok()?;
        let file: ClaudeCredentialsFile = serde_json::from_str(&content).ok()?;
        let token = file.claude_ai_oauth?.access_token?;
        (!token.is_empty()).then_some(token)
    })
}

fn claude_config_dirs() -> Vec<PathBuf> {
    if let Ok(paths) = env::var("CLAUDE_CONFIG_DIR") {
        return paths
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    home::home_dir()
        .map(|home| vec![home.join(".config").join("claude"), home.join(".claude")])
        .unwrap_or_default()
}

// --- Codex -----------------------------------------------------------------

struct CodexAuth {
    access_token: String,
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct CodexAuthFile {
    tokens: Option<CodexAuthTokens>,
}

#[derive(Deserialize)]
struct CodexAuthTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

/// `GET https://chatgpt.com/backend-api/wham/usage` response. The primary
/// window is the 5-hour session limit, the secondary window the weekly one.
#[derive(Deserialize)]
struct CodexUsageResponse {
    plan_type: Option<String>,
    rate_limit: Option<CodexRateLimit>,
}

#[derive(Deserialize)]
struct CodexRateLimit {
    primary_window: Option<CodexRateLimitWindow>,
    secondary_window: Option<CodexRateLimitWindow>,
}

#[derive(Deserialize)]
struct CodexRateLimitWindow {
    used_percent: Option<f64>,
    limit_window_seconds: Option<i64>,
    reset_after_seconds: Option<i64>,
    /// Unix epoch seconds.
    reset_at: Option<i64>,
}

fn codex_limits() -> AgentOutcome {
    let Some(auth) = codex_auth() else {
        return AgentOutcome::unavailable(
            "codex",
            "Codex",
            "no ChatGPT OAuth credentials found (sign in with Codex CLI so ~/.codex/auth.json contains tokens.access_token)",
        );
    };
    let user_agent = format!("turbotokens/{}", env!("TURBOTOKENS_VERSION"));
    let authorization = format!("Bearer {}", auth.access_token);
    let account_id = auth.account_id.unwrap_or_default();
    let mut headers: Vec<(&str, &str)> = vec![
        ("Authorization", authorization.as_str()),
        ("User-Agent", user_agent.as_str()),
        ("Accept", "application/json"),
    ];
    if !account_id.is_empty() {
        headers.push(("ChatGPT-Account-Id", account_id.as_str()));
    }
    let body = match crate::http::fetch_json_with_headers(CODEX_USAGE_URL, &headers) {
        Ok(body) => body,
        Err(error) => {
            return AgentOutcome::unavailable(
                "codex",
                "Codex",
                fetch_error_reason(CODEX_USAGE_URL, &error, "codex"),
            );
        }
    };
    match parse_codex_usage(&body, utc_now()) {
        Ok(limits) => AgentOutcome::Available(limits),
        Err(error) => AgentOutcome::unavailable("codex", "Codex", error.to_string()),
    }
}

fn parse_codex_usage(body: &str, now: TimestampMs) -> Result<AgentLimits> {
    let response: CodexUsageResponse = serde_json::from_str(body)
        .context("Unexpected response shape from the Codex usage endpoint")?;
    let mut windows = Vec::new();
    if let Some(rate_limit) = response.rate_limit {
        for (name, fallback_label, window) in [
            ("primary_window", "5-hour window", rate_limit.primary_window),
            (
                "secondary_window",
                "Weekly window",
                rate_limit.secondary_window,
            ),
        ] {
            if let Some(window) =
                window.and_then(|window| codex_window(name, fallback_label, window, now))
            {
                windows.push(window);
            }
        }
    }
    if windows.is_empty() {
        return Err(cli_error(
            "the Codex usage endpoint response contained no rate_limit windows; the API shape may have changed",
        ));
    }
    Ok(AgentLimits {
        agent: "codex",
        title: "Codex",
        plan: response.plan_type,
        windows,
    })
}

fn codex_window(
    name: &'static str,
    fallback_label: &str,
    window: CodexRateLimitWindow,
    now: TimestampMs,
) -> Option<WindowLimit> {
    let utilization = window.used_percent?;
    let label = match window.limit_window_seconds {
        Some(18_000) => "5-hour window".to_string(),
        Some(604_800) => "Weekly window".to_string(),
        Some(seconds) if seconds > 0 => format!("{}-hour window", seconds / 3_600),
        _ => fallback_label.to_string(),
    };
    let resets_at = window
        .reset_at
        .and_then(TimestampMs::from_unix_seconds)
        .or_else(|| {
            window
                .reset_after_seconds
                .and_then(|seconds| now.checked_add_millis(seconds.saturating_mul(1_000)))
        });
    Some(WindowLimit {
        name,
        label,
        utilization_percent: utilization,
        resets_at,
    })
}

fn codex_auth() -> Option<CodexAuth> {
    codex_homes().into_iter().find_map(|home| {
        let content = fs::read_to_string(home.join("auth.json")).ok()?;
        let file: CodexAuthFile = serde_json::from_str(&content).ok()?;
        let tokens = file.tokens?;
        let access_token = tokens.access_token.filter(|token| !token.is_empty())?;
        Some(CodexAuth {
            access_token,
            account_id: tokens.account_id,
        })
    })
}

fn codex_homes() -> Vec<PathBuf> {
    if let Ok(env_paths) = env::var("CODEX_HOME") {
        return env_paths
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    home::home_dir()
        .map(|home| vec![home.join(".codex")])
        .unwrap_or_default()
}

/// Turns a transport/HTTP failure into an actionable reason. 401/403 mean the
/// stored OAuth token is stale, so point at the agent's own re-login.
fn fetch_error_reason(url: &str, error: &std::io::Error, agent: &str) -> String {
    let message = error.to_string();
    if message.contains("HTTP 401") || message.contains("HTTP 403") {
        format!(
            "{url} rejected the stored OAuth token ({message}); re-authenticate with `{agent}` to refresh it"
        )
    } else {
        format!("failed to fetch {url}: {message}")
    }
}

// --- Rendering ---------------------------------------------------------------

const BAR_WIDTH: usize = 20;

fn utilization_color(percent: f64) -> Color {
    if percent < 50.0 {
        Color::Green
    } else if percent < 80.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

fn utilization_bar(percent: f64) -> String {
    let clamped = percent.clamp(0.0, 100.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let filled = ((clamped / 100.0) * BAR_WIDTH as f64).round() as usize;
    format!(
        "[{}{}]",
        "█".repeat(filled),
        "░".repeat(BAR_WIDTH - filled.min(BAR_WIDTH))
    )
}

fn print_text_report(outcomes: &[AgentOutcome], shared: &SharedArgs, now: TimestampMs) {
    let mut first = true;
    for outcome in outcomes {
        if !first {
            println!();
        }
        first = false;
        match outcome {
            AgentOutcome::Available(limits) => print_agent_limits(limits, shared, now),
            AgentOutcome::Unavailable { title, reason, .. } => {
                println!("{title}: {reason}");
            }
        }
    }
}

fn print_agent_limits(limits: &AgentLimits, shared: &SharedArgs, now: TimestampMs) {
    let plan = limits
        .plan
        .as_deref()
        .map(|plan| format!(" ({plan} plan)"))
        .unwrap_or_default();
    println!("{} plan limits{plan}", limits.title);
    for window in &limits.windows {
        println!("  {}", window_line(window, shared, now));
    }
}

fn window_line(window: &WindowLimit, shared: &SharedArgs, now: TimestampMs) -> String {
    let percent = window.utilization_percent;
    let bar = color(shared, utilization_bar(percent), utilization_color(percent));
    let reset = match window.resets_at {
        Some(resets_at) if resets_at.as_millis() > now.as_millis() => {
            let minutes = resets_at.duration_since(now) / MILLIS_PER_MINUTE;
            format!(
                "resets {} ({})",
                format_minute(resets_at, shared.timezone.as_deref()),
                format_remaining_time(minutes)
            )
        }
        Some(resets_at) => format!(
            "reset due since {}",
            format_minute(resets_at, shared.timezone.as_deref())
        ),
        None => "reset time unknown".to_string(),
    };
    format!(
        "{:<17} {} {:>5.1}% used   {}",
        window.label, bar, percent, reset
    )
}

fn report_json(outcomes: &[AgentOutcome], now: TimestampMs) -> Value {
    json!({
        "agents": outcomes.iter().map(|outcome| outcome_json(outcome, now)).collect::<Vec<_>>(),
    })
}

fn outcome_json(outcome: &AgentOutcome, now: TimestampMs) -> Value {
    match outcome {
        AgentOutcome::Available(limits) => json!({
            "agent": limits.agent,
            "ok": true,
            "error": null,
            "plan": limits.plan,
            "windows": limits.windows.iter().map(|window| window_json(window, now)).collect::<Vec<_>>(),
        }),
        AgentOutcome::Unavailable { agent, reason, .. } => json!({
            "agent": agent,
            "ok": false,
            "error": reason,
            "plan": null,
            "windows": [],
        }),
    }
}

fn window_json(window: &WindowLimit, now: TimestampMs) -> Value {
    json!({
        "name": window.name,
        "label": window.label,
        "utilizationPercent": window.utilization_percent,
        "resetsAt": window.resets_at.map(format_rfc3339_millis),
        "resetsInSeconds": window
            .resets_at
            .filter(|resets_at| resets_at.as_millis() > now.as_millis())
            .map(|resets_at| resets_at.duration_since(now) / 1_000),
    })
}

#[cfg(test)]
mod tests {
    use turbotokens_test_support::{EnvVarsGuard, fs_fixture};

    use super::*;

    const NOW: i64 = 1_776_700_000_000; // fixed point in time for fixtures

    fn now() -> TimestampMs {
        TimestampMs::from_millis(NOW)
    }

    fn no_color_shared() -> SharedArgs {
        SharedArgs {
            no_color: true,
            timezone: Some("UTC".to_string()),
            ..SharedArgs::default()
        }
    }

    #[test]
    fn parses_claude_usage_response_with_all_windows() {
        let body = r#"{
            "five_hour": { "utilization": 58, "resets_at": "2026-04-14T10:00:00+00:00" },
            "seven_day": { "utilization": 10.5, "resets_at": "2026-04-20T23:00:00.000Z" },
            "seven_day_opus": { "utilization": 3, "resets_at": "2026-04-20T23:00:00Z" }
        }"#;

        let limits = parse_claude_usage(body).unwrap();

        assert_eq!(limits.agent, "claude");
        assert_eq!(limits.windows.len(), 3);
        assert_eq!(limits.windows[0].name, "five_hour");
        assert_eq!(limits.windows[0].utilization_percent, 58.0);
        assert_eq!(
            limits.windows[0].resets_at.map(format_rfc3339_millis),
            Some("2026-04-14T10:00:00.000Z".to_string())
        );
        assert_eq!(limits.windows[1].utilization_percent, 10.5);
        assert_eq!(limits.windows[2].name, "seven_day_opus");
    }

    #[test]
    fn parses_claude_usage_response_without_optional_windows() {
        let body = r#"{
            "five_hour": { "utilization": 0, "resets_at": null },
            "seven_day": null
        }"#;

        let limits = parse_claude_usage(body).unwrap();

        assert_eq!(limits.windows.len(), 1);
        assert_eq!(limits.windows[0].resets_at, None);
    }

    #[test]
    fn rejects_claude_usage_response_without_windows() {
        let error = parse_claude_usage("{}").unwrap_err();
        assert!(error.to_string().contains("no known windows"));

        let error = parse_claude_usage("{\"five_hour\": {\"used\": 42}}").unwrap_err();
        assert!(error.to_string().contains("no known windows"));

        let error = parse_claude_usage("<html>cloudflare</html>").unwrap_err();
        assert!(error.to_string().contains("Unexpected response shape"));
    }

    #[test]
    fn parses_codex_usage_response() {
        let body = r#"{
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": { "used_percent": 42, "limit_window_seconds": 18000, "reset_after_seconds": 0, "reset_at": 1776703600 },
                "secondary_window": { "used_percent": 84, "limit_window_seconds": 604800, "reset_after_seconds": 0, "reset_at": 1777300000 }
            },
            "credits": { "has_credits": true, "unlimited": false, "balance": "9.99" }
        }"#;

        let limits = parse_codex_usage(body, now()).unwrap();

        assert_eq!(limits.agent, "codex");
        assert_eq!(limits.plan.as_deref(), Some("pro"));
        assert_eq!(limits.windows.len(), 2);
        assert_eq!(limits.windows[0].name, "primary_window");
        assert_eq!(limits.windows[0].label, "5-hour window");
        assert_eq!(limits.windows[0].utilization_percent, 42.0);
        assert_eq!(
            limits.windows[0].resets_at,
            TimestampMs::from_unix_seconds(1_776_703_600)
        );
        assert_eq!(limits.windows[1].label, "Weekly window");
    }

    #[test]
    fn codex_window_falls_back_to_reset_after_seconds() {
        let body = r#"{
            "rate_limit": {
                "primary_window": { "used_percent": 12.5, "limit_window_seconds": 18000, "reset_after_seconds": 3600, "reset_at": null },
                "secondary_window": null
            }
        }"#;

        let limits = parse_codex_usage(body, now()).unwrap();

        assert_eq!(limits.windows.len(), 1);
        assert_eq!(
            limits.windows[0].resets_at,
            now().checked_add_millis(3_600_000)
        );
    }

    #[test]
    fn rejects_codex_usage_response_without_windows() {
        let error = parse_codex_usage("{\"plan_type\": \"pro\"}", now()).unwrap_err();
        assert!(error.to_string().contains("no rate_limit windows"));

        let error = parse_codex_usage("not json", now()).unwrap_err();
        assert!(error.to_string().contains("Unexpected response shape"));
    }

    #[test]
    fn reads_claude_token_from_credentials_file() {
        let fixture = fs_fixture!({
            ".credentials.json": r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat-fixture", "refreshToken": "ignored"}}"#,
        });
        let _env = EnvVarsGuard::set_many([
            ("CLAUDE_CODE_OAUTH_TOKEN", None),
            (
                "CLAUDE_CONFIG_DIR",
                Some(fixture.root().as_os_str().to_os_string()),
            ),
        ]);

        assert_eq!(claude_access_token().as_deref(), Some("sk-ant-oat-fixture"));
    }

    #[test]
    fn claude_env_token_wins_over_credentials_file() {
        let fixture = fs_fixture!({
            ".credentials.json": r#"{"claudeAiOauth": {"accessToken": "from-file"}}"#,
        });
        let _env = EnvVarsGuard::set_many([
            ("CLAUDE_CODE_OAUTH_TOKEN", Some("from-env".into())),
            (
                "CLAUDE_CONFIG_DIR",
                Some(fixture.root().as_os_str().to_os_string()),
            ),
        ]);

        assert_eq!(claude_access_token().as_deref(), Some("from-env"));
    }

    #[test]
    fn claude_token_is_none_without_credentials() {
        let fixture = fs_fixture!({});
        let _env = EnvVarsGuard::set_many([
            ("CLAUDE_CODE_OAUTH_TOKEN", None),
            (
                "CLAUDE_CONFIG_DIR",
                Some(fixture.root().as_os_str().to_os_string()),
            ),
        ]);

        assert_eq!(claude_access_token(), None);
    }

    #[test]
    fn reads_codex_auth_from_auth_json() {
        let fixture = fs_fixture!({
            "auth.json": r#"{"tokens": {"access_token": "codex-fixture-token", "account_id": "acct-123"}, "OPENAI_API_KEY": null}"#,
        });
        let _env = EnvVarsGuard::set_many([(
            "CODEX_HOME",
            Some(fixture.root().as_os_str().to_os_string()),
        )]);

        let auth = codex_auth().unwrap();
        assert_eq!(auth.access_token, "codex-fixture-token");
        assert_eq!(auth.account_id.as_deref(), Some("acct-123"));
    }

    #[test]
    fn codex_auth_is_none_for_api_key_only_login() {
        let fixture = fs_fixture!({
            "auth.json": r#"{"OPENAI_API_KEY": "sk-fixture"}"#,
        });
        let _env = EnvVarsGuard::set_many([(
            "CODEX_HOME",
            Some(fixture.root().as_os_str().to_os_string()),
        )]);

        assert!(codex_auth().is_none());
    }

    #[test]
    fn window_line_renders_bar_percentage_and_reset() {
        let window = WindowLimit {
            name: "five_hour",
            label: "5-hour window".to_string(),
            utilization_percent: 58.0,
            resets_at: now().checked_add_millis(83 * MILLIS_PER_MINUTE),
        };

        let line = window_line(&window, &no_color_shared(), now());

        assert!(line.contains("5-hour window"));
        assert!(line.contains("58.0% used"));
        assert!(line.contains("1h 23m left"));
        assert!(line.contains("[████████████░░░░░░░░]"), "line: {line}");
        assert!(!line.contains('\x1b'));
    }

    #[test]
    fn json_report_carries_all_fields_and_errors() {
        let outcomes = vec![
            AgentOutcome::Available(AgentLimits {
                agent: "claude",
                title: "Claude Code",
                plan: None,
                windows: vec![WindowLimit {
                    name: "five_hour",
                    label: "5-hour window".to_string(),
                    utilization_percent: 58.0,
                    resets_at: now().checked_add_millis(3_600_000),
                }],
            }),
            AgentOutcome::unavailable("codex", "Codex", "no credentials"),
        ];

        let report = report_json(&outcomes, now());

        let claude = &report["agents"][0];
        assert_eq!(claude["agent"], json!("claude"));
        assert_eq!(claude["ok"], json!(true));
        assert_eq!(claude["error"], Value::Null);
        assert_eq!(claude["windows"][0]["utilizationPercent"], json!(58.0));
        assert_eq!(claude["windows"][0]["resetsInSeconds"], json!(3600));
        assert_eq!(
            claude["windows"][0]["resetsAt"],
            json!(format_rfc3339_millis(
                now().checked_add_millis(3_600_000).unwrap()
            ))
        );

        let codex = &report["agents"][1];
        assert_eq!(codex["ok"], json!(false));
        assert_eq!(codex["error"], json!("no credentials"));
        assert_eq!(codex["windows"], json!([]));
    }

    #[test]
    fn utilization_colors_follow_thresholds() {
        assert_eq!(utilization_color(49.9), Color::Green);
        assert_eq!(utilization_color(50.0), Color::Yellow);
        assert_eq!(utilization_color(80.0), Color::Red);
        assert_eq!(utilization_bar(0.0), "[░░░░░░░░░░░░░░░░░░░░]");
        assert_eq!(utilization_bar(100.0), "[████████████████████]");
    }
}
