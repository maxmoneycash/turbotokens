use std::ffi::OsString;

use serde_json::{Value, json};

use crate::help::{help_text, help_text_for_args};
use crate::*;
use turbotokens_cli::*;
use turbotokens_test_support::fs_fixture;

fn parse(args: &[&str]) -> Cli {
    Cli::parse_from(args.iter().map(OsString::from)).unwrap()
}

fn parse_with_config(args: &[&str], config: &dyn CliConfig) -> Cli {
    Cli::parse_from_with_config(
        args.iter().map(OsString::from),
        config,
        5.0,
        env!("CARGO_PKG_VERSION"),
    )
    .unwrap()
}

fn parse_error(args: &[&str]) -> String {
    match Cli::parse_from(args.iter().map(OsString::from)) {
        Ok(_) => panic!("expected parse error"),
        Err(error) => error,
    }
}

#[derive(Default)]
struct TestConfig {
    shared_json: Option<bool>,
    shared_order: Option<SortOrder>,
    shared_since: Option<&'static str>,
    shared_timezone: Option<&'static str>,
    shared_compact: Option<bool>,
    weekly_start: Option<WeekDay>,
    blocks_active: Option<bool>,
    blocks_token_limit: Option<&'static str>,
    blocks_session_length: Option<f64>,
    statusline_visual_burn_rate: Option<VisualBurnRate>,
    statusline_cost_source: Option<CostSource>,
    statusline_refresh_interval: Option<u64>,
    codex_speed: Option<CodexSpeed>,
    pi_path: Option<&'static str>,
    open_claw_path: Option<&'static str>,
}

impl CliConfig for TestConfig {
    fn apply_shared(&self, shared: &mut SharedArgs) {
        if let Some(json) = self.shared_json {
            shared.json = json;
        }
        if let Some(order) = self.shared_order {
            shared.order = order;
        }
        if let Some(since) = self.shared_since {
            shared.since = Some(since.to_string());
        }
        if let Some(timezone) = self.shared_timezone {
            shared.timezone = Some(timezone.to_string());
        }
        if let Some(compact) = self.shared_compact {
            shared.compact = compact;
        }
    }

    fn apply_weekly_args(&self, args: &mut WeeklyArgs) {
        if let Some(start_of_week) = self.weekly_start {
            args.start_of_week = start_of_week;
        }
    }

    fn apply_blocks_args(&self, args: &mut BlocksArgs) {
        if let Some(active) = self.blocks_active {
            args.active = active;
        }
        if let Some(token_limit) = self.blocks_token_limit {
            args.token_limit = Some(token_limit.to_string());
        }
        if let Some(session_length) = self.blocks_session_length {
            args.session_length = session_length;
        }
    }

    fn apply_statusline_args(&self, args: &mut StatuslineArgs) {
        if let Some(visual_burn_rate) = self.statusline_visual_burn_rate {
            args.visual_burn_rate = visual_burn_rate;
        }
        if let Some(cost_source) = self.statusline_cost_source {
            args.cost_source = cost_source;
        }
        if let Some(refresh_interval) = self.statusline_refresh_interval {
            args.refresh_interval = refresh_interval;
        }
    }

    fn apply_agent_args(
        &self,
        codex_speed: &mut CodexSpeed,
        pi_path: Option<&mut Option<String>>,
        open_claw_path: Option<&mut Option<String>>,
    ) {
        if let Some(speed) = self.codex_speed {
            *codex_speed = speed;
        }
        if let (Some(path), Some(pi_path)) = (self.pi_path, pi_path) {
            *pi_path = Some(path.to_string());
        }
        if let (Some(path), Some(open_claw_path)) = (self.open_claw_path, open_claw_path) {
            *open_claw_path = Some(path.to_string());
        }
    }
}

fn shared_snapshot(shared: &SharedArgs) -> Value {
    json!({
        "since": shared.since.as_deref(),
        "until": shared.until.as_deref(),
        "json": shared.json,
        "mode": format!("{:?}", shared.mode),
        "debug": shared.debug,
        "debugSamples": shared.debug_samples,
        "order": format!("{:?}", shared.order),
        "breakdown": shared.breakdown,
        "offline": shared.offline,
        "noOffline": shared.no_offline,
        "color": shared.color,
        "noColor": shared.no_color,
        "timezone": shared.timezone.as_deref(),
        "jq": shared.jq.as_deref(),
        "config": shared.config.as_ref().map(|path| path.to_string_lossy().to_string()),
        "compact": shared.compact,
        "singleThread": shared.single_thread,
    })
}

fn cli_snapshot(cli: Cli) -> Value {
    json!({
        "shared": shared_snapshot(&cli.shared),
        "command": command_snapshot(cli.command),
    })
}

fn command_snapshot(command: Option<Command>) -> Value {
    match command {
        None => Value::Null,
        Some(Command::All(args)) => agent_command_snapshot("all", args),
        Some(Command::Daily(args)) => json!({
            "type": "daily",
            "shared": shared_snapshot(&args.shared),
            "instances": args.instances,
            "project": args.project,
            "projectAliases": args.project_aliases,
            "watch": args.watch,
        }),
        Some(Command::Monthly(shared)) => json!({
            "type": "monthly",
            "shared": shared_snapshot(&shared),
        }),
        Some(Command::Weekly(args)) => json!({
            "type": "weekly",
            "shared": shared_snapshot(&args.shared),
            "startOfWeek": format!("{:?}", args.start_of_week),
        }),
        Some(Command::Session(args)) => json!({
            "type": "session",
            "shared": shared_snapshot(&args.shared),
            "id": args.id,
        }),
        Some(Command::Blocks(args)) => json!({
            "type": "blocks",
            "shared": shared_snapshot(&args.shared),
            "active": args.active,
            "recent": args.recent,
            "tokenLimit": args.token_limit,
            "sessionLength": args.session_length,
            "watch": args.watch,
        }),
        Some(Command::Statusline(args)) => json!({
            "type": "statusline",
            "offline": args.offline,
            "noOffline": args.no_offline,
            "visualBurnRate": format!("{:?}", args.visual_burn_rate),
            "costSource": format!("{:?}", args.cost_source),
            "cache": args.cache,
            "noCache": args.no_cache,
            "refreshInterval": args.refresh_interval,
            "contextLowThreshold": args.context_low_threshold,
            "contextMediumThreshold": args.context_medium_threshold,
            "config": args.config.as_ref().map(|path| path.to_string_lossy().to_string()),
            "debug": args.debug,
        }),
        Some(Command::Codex(args)) => agent_command_snapshot("codex", args),
        Some(Command::Live(args)) => json!({
            "type": "live",
            "shared": shared_snapshot(&args.shared),
            "intervalMs": args.interval_ms,
            "alertCost": args.alert_cost,
            "alertTokens": args.alert_tokens,
            "webhook": args.webhook,
            "serve": args.serve,
            "agent": format!("{:?}", args.agent),
        }),
        Some(Command::Daemon(args)) => json!({
            "type": "daemon",
            "shared": shared_snapshot(&args.shared),
            "action": format!("{:?}", args.action),
            "intervalMs": args.interval_ms,
        }),
        Some(Command::Doctor(shared)) => json!({
            "type": "doctor",
            "shared": shared_snapshot(&shared),
        }),
        Some(Command::Completions(args)) => json!({
            "type": "completions",
            "shell": format!("{:?}", args.shell),
        }),
        Some(Command::OpenCode(args)) => agent_command_snapshot("opencode", args),
        Some(Command::Amp(args)) => agent_command_snapshot("amp", args),
        Some(Command::Droid(args)) => agent_command_snapshot("droid", args),
        Some(Command::Codebuff(args)) => agent_command_snapshot("codebuff", args),
        Some(Command::Hermes(args)) => agent_command_snapshot("hermes", args),
        Some(Command::Pi(args)) => agent_command_snapshot("pi", args),
        Some(Command::Goose(args)) => agent_command_snapshot("goose", args),
        Some(Command::Grok(args)) => agent_command_snapshot("grok", args),
        Some(Command::Kilo(args)) => agent_command_snapshot("kilo", args),
        Some(Command::Copilot(args)) => agent_command_snapshot("copilot", args),
        Some(Command::Gemini(args)) => agent_command_snapshot("gemini", args),
        Some(Command::Antigravity(args)) => agent_command_snapshot("antigravity", args),
        Some(Command::Kimi(args)) => agent_command_snapshot("kimi", args),
        Some(Command::Qwen(args)) => agent_command_snapshot("qwen", args),
        Some(Command::OpenClaw(args)) => agent_command_snapshot("openclaw", args),
        Some(Command::ZCode(args)) => agent_command_snapshot("zcode", args),
        Some(Command::Limits(args)) => json!({
            "type": "limits",
            "shared": shared_snapshot(&args.shared),
            "scope": format!("{:?}", args.scope),
        }),
        Some(Command::Import(args)) => json!({
            "type": "import",
            "shared": shared_snapshot(&args.shared),
            "file": args.file.to_string_lossy().to_string(),
        }),
        Some(Command::Heatmap(args)) => json!({
            "type": "heatmap",
            "shared": shared_snapshot(&args.shared),
            "cost": args.cost,
            "svg": args.svg,
        }),
        Some(Command::Wrapped(args)) => json!({
            "type": "wrapped",
            "shared": shared_snapshot(&args.shared),
            "year": args.year,
            "svg": args.svg,
        }),
    }
}

fn agent_command_snapshot(agent: &str, args: AgentCommandArgs) -> Value {
    json!({
        "type": agent,
        "shared": shared_snapshot(&args.shared),
        "kind": format!("{:?}", args.kind),
        "sections": args.sections.map(|sections| sections
            .into_iter()
            .map(|section| format!("{section:?}"))
            .collect::<Vec<_>>()),
        "byAgent": args.by_agent,
        "piPath": args.pi_path,
        "openClawPath": args.open_claw_path,
        "codexSpeed": format!("{:?}", args.codex_speed),
    })
}

#[test]
fn parses_live_command_with_default_interval() {
    let cli = parse(&["turbotokens", "live"]);
    let Some(Command::Live(args)) = cli.command else {
        panic!("expected live command");
    };
    assert_eq!(args.interval_ms, 100);

    let cli = parse(&["turbotokens", "live", "--json", "--interval", "250"]);
    let Some(Command::Live(args)) = cli.command else {
        panic!("expected live command");
    };
    assert_eq!(args.interval_ms, 250);
    assert!(args.shared.json);

    assert!(parse_error(&["turbotokens", "live", "--interval", "0"]).contains("--interval"));
    assert!(parse_error(&["turbotokens", "live", "--bogus"]).contains("Unknown live option"));
}

#[test]
fn stream_is_live_with_json_forced() {
    let cli = parse(&["turbotokens", "stream"]);
    let Some(Command::Live(args)) = cli.command else {
        panic!("expected live command");
    };
    assert!(args.shared.json);
    assert_eq!(args.interval_ms, 100);
    assert_eq!(args.agent, LiveAgent::Claude);

    let cli = parse(&[
        "turbotokens",
        "stream",
        "--agent",
        "codex",
        "--interval",
        "250",
        "--serve",
        "127.0.0.1:9090",
    ]);
    let Some(Command::Live(args)) = cli.command else {
        panic!("expected live command");
    };
    assert!(args.shared.json);
    assert_eq!(args.agent, LiveAgent::Codex);
    assert_eq!(args.interval_ms, 250);
    assert_eq!(args.serve.as_deref(), Some("127.0.0.1:9090"));
}

#[test]
fn parses_live_agent_alert_webhook_and_serve_options() {
    let cli = parse(&[
        "turbotokens",
        "live",
        "--agent",
        "codex",
        "--alert-cost",
        "1.5",
        "--alert-tokens",
        "1000",
        "--webhook",
        "http://127.0.0.1:9/hook",
        "--serve",
        "127.0.0.1:9090",
    ]);
    let Some(Command::Live(args)) = cli.command else {
        panic!("expected live command");
    };
    assert_eq!(args.agent, LiveAgent::Codex);
    assert_eq!(args.alert_cost, Some(1.5));
    assert_eq!(args.alert_tokens, Some(1000));
    assert_eq!(args.webhook.as_deref(), Some("http://127.0.0.1:9/hook"));
    assert_eq!(args.serve.as_deref(), Some("127.0.0.1:9090"));

    let cli = parse(&["turbotokens", "live", "--agent", "claude"]);
    let Some(Command::Live(args)) = cli.command else {
        panic!("expected live command");
    };
    assert_eq!(args.agent, LiveAgent::Claude);

    let cli = parse(&["turbotokens", "live", "--agent", "grok"]);
    let Some(Command::Live(args)) = cli.command else {
        panic!("expected live command");
    };
    assert_eq!(args.agent, LiveAgent::Grok);

    assert!(
        parse_error(&["turbotokens", "live", "--agent", "gemini"]).contains("Unknown live agent")
    );
}

#[test]
fn parses_root_daily_as_all_agent_report() {
    let cli = parse(&["turbotokens", "daily", "--json", "--since", "20260102"]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(args.kind, AgentReportKind::Daily);
    assert!(args.shared.json);
    assert_eq!(args.shared.since.as_deref(), Some("20260102"));
}

#[test]
fn parses_last_periods_on_period_reports() {
    let cli = parse(&["turbotokens", "daily", "--last", "1"]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(args.shared.last, Some(1));

    let cli = parse(&["turbotokens", "claude", "weekly", "--last", "2"]);
    let Some(Command::Weekly(args)) = cli.command else {
        panic!("expected weekly command");
    };
    assert_eq!(args.shared.last, Some(2));

    let cli = parse(&["turbotokens", "codex", "monthly", "--last=3"]);
    let Some(Command::Codex(args)) = cli.command else {
        panic!("expected codex command");
    };
    assert_eq!(args.shared.last, Some(3));
}

#[test]
fn parses_last_periods_before_the_command_name() {
    let cli = parse(&["turbotokens", "--last", "1", "weekly"]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(args.kind, AgentReportKind::Weekly);
    assert_eq!(args.shared.last, Some(1));

    // The bare command is the unified daily report, which the binary resolves
    // from the root shared options.
    let cli = parse(&["turbotokens", "--last", "1"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.shared.last, Some(1));
}

#[test]
fn leaves_the_short_alias_of_the_removed_locale_option_unused() {
    assert_eq!(
        parse_error(&["turbotokens", "daily", "-l", "1"]),
        "Unknown option '-l'"
    );
}

#[test]
fn rejects_last_periods_on_reports_without_a_period() {
    for args in [
        ["turbotokens", "session", "--last", "1"],
        ["turbotokens", "blocks", "--last", "1"],
        ["turbotokens", "codex", "session", "--last=1"],
    ] {
        assert_eq!(
            parse_error(&args),
            "The --last option is only available for the daily, weekly, and monthly reports."
        );
    }
}

#[test]
fn rejects_last_periods_alongside_an_explicit_date_window() {
    assert_eq!(
        parse_error(&["turbotokens", "daily", "--last", "1", "--since", "20260101"]),
        "The --last option cannot be combined with --since or --until."
    );
    assert_eq!(
        parse_error(&["turbotokens", "daily", "--last", "1", "--until", "20260101"]),
        "The --last option cannot be combined with --since or --until."
    );
}

#[test]
fn rejects_last_periods_with_multi_section_reports() {
    assert_eq!(
        parse_error(&[
            "turbotokens",
            "daily",
            "--last",
            "1",
            "--sections",
            "monthly"
        ]),
        "The --last option cannot be used with --sections."
    );
}

#[test]
fn rejects_last_periods_that_are_not_positive_whole_numbers() {
    assert_eq!(
        parse_error(&["turbotokens", "daily", "--last", "0"]),
        "Invalid value for --last '0'. Expected a whole number of periods, 1 or greater."
    );
    assert_eq!(
        parse_error(&["turbotokens", "daily", "--last", "week"]),
        "Invalid value for --last 'week'. Expected a whole number of periods, 1 or greater."
    );
}

#[test]
fn parses_unified_sections_and_by_agent_flags() {
    let cli = parse(&[
        "turbotokens",
        "daily",
        "--json",
        "--sections",
        "monthly,session",
        "--by-agent",
    ]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(args.kind, AgentReportKind::Daily);
    assert_eq!(
        args.sections.as_deref(),
        Some(&[AgentReportKind::Monthly, AgentReportKind::Session][..])
    );
    assert!(args.by_agent);
}

#[test]
fn parses_root_sections_and_by_agent_flags_without_daily_token() {
    let cli = parse(&[
        "turbotokens",
        "--json",
        "--sections",
        "monthly,session",
        "--by-agent",
    ]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(args.kind, AgentReportKind::Daily);
    assert_eq!(
        args.sections.as_deref(),
        Some(&[AgentReportKind::Monthly, AgentReportKind::Session][..])
    );
    assert!(args.by_agent);
}

#[test]
fn parses_inline_unified_sections_value() {
    let cli = parse(&["turbotokens", "daily", "--sections=daily,monthly"]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(
        args.sections.as_deref(),
        Some(&[AgentReportKind::Daily, AgentReportKind::Monthly][..])
    );
}

#[test]
fn ignores_empty_unified_section_tokens() {
    let cli = parse(&["turbotokens", "daily", "--sections", "daily,,monthly,"]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(
        args.sections.as_deref(),
        Some(&[AgentReportKind::Daily, AgentReportKind::Monthly][..])
    );
}

#[test]
fn dedupes_unified_section_tokens_preserving_first_occurrence() {
    let cli = parse(&["turbotokens", "daily", "--sections", "daily,daily,monthly"]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(
        args.sections.as_deref(),
        Some(&[AgentReportKind::Daily, AgentReportKind::Monthly][..])
    );
}

#[test]
fn parses_top_level_session_sections_as_all_agent_report_without_id() {
    let cli = parse(&[
        "turbotokens",
        "session",
        "--sections",
        "daily,weekly",
        "--by-agent",
    ]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert_eq!(
        args.sections.as_deref(),
        Some(&[AgentReportKind::Daily, AgentReportKind::Weekly][..])
    );
    assert!(args.by_agent);
}

#[test]
fn rejects_sections_and_by_agent_with_top_level_session_id() {
    let sections_error = parse_error(&[
        "turbotokens",
        "session",
        "--id",
        "abc",
        "--sections",
        "daily",
    ]);
    assert_eq!(
        sections_error,
        "The --sections and --by-agent options cannot be used with session --id."
    );

    let by_agent_error = parse_error(&["turbotokens", "session", "--id", "abc", "--by-agent"]);
    assert_eq!(
        by_agent_error,
        "The --sections and --by-agent options cannot be used with session --id."
    );
}

#[test]
fn rejects_unified_only_flags_on_per_agent_subcommands() {
    let sections_error = parse_error(&["turbotokens", "codex", "daily", "--sections", "daily"]);
    assert_eq!(sections_error, "Unknown codex option '--sections'");

    let by_agent_error = parse_error(&["turbotokens", "codex", "daily", "--by-agent"]);
    assert_eq!(by_agent_error, "Unknown codex option '--by-agent'");
}

#[test]
fn rejects_invalid_unified_section_token() {
    let error = parse_error(&["turbotokens", "daily", "--sections", "daily,yearly"]);

    assert_eq!(
        error,
        "Invalid --sections value 'yearly'. Expected one or more of: daily, weekly, monthly, session."
    );
}

#[test]
fn preserves_shared_config_defaults_with_unified_sections() {
    let config = TestConfig {
        shared_json: Some(true),
        shared_since: Some("20260102"),
        ..TestConfig::default()
    };

    let cli = parse_with_config(&["turbotokens", "monthly", "--sections", "daily"], &config);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert!(args.shared.json);
    assert_eq!(args.shared.since.as_deref(), Some("20260102"));
    assert_eq!(
        args.sections.as_deref(),
        Some(&[AgentReportKind::Daily][..])
    );
}

#[test]
fn parses_root_session_as_all_agent_report_without_id() {
    let cli = parse(&["turbotokens", "session", "--json"]);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn applies_config_defaults_and_command_options_before_cli_options() {
    let config = TestConfig {
        shared_json: Some(true),
        shared_order: Some(SortOrder::Desc),
        shared_since: Some("20260102"),
        ..TestConfig::default()
    };

    let cli = parse_with_config(&["turbotokens", "daily", "--order", "asc"], &config);
    let Some(Command::All(args)) = cli.command else {
        panic!("expected all-agent command");
    };
    assert!(args.shared.json);
    assert_eq!(args.shared.since.as_deref(), Some("20260102"));
    assert_eq!(args.shared.order, SortOrder::Asc);
}

#[test]
fn applies_agent_namespace_config_to_codex_speed() {
    let config = TestConfig {
        codex_speed: Some(CodexSpeed::Fast),
        ..TestConfig::default()
    };

    let cli = parse_with_config(&["turbotokens", "codex", "daily"], &config);
    let Some(Command::Codex(args)) = cli.command else {
        panic!("expected codex command");
    };
    assert_eq!(args.codex_speed, CodexSpeed::Fast);
}

#[test]
fn applies_config_file_passed_after_agent_command() {
    let config = TestConfig {
        shared_json: Some(true),
        shared_timezone: Some("Asia/Tokyo"),
        shared_since: Some("20260101"),
        codex_speed: Some(CodexSpeed::Standard),
        ..TestConfig::default()
    };

    let cli = parse_with_config(
        &[
            "turbotokens",
            "codex",
            "monthly",
            "--config",
            "/tmp/turbotokens.json",
        ],
        &config,
    );
    let Some(Command::Codex(args)) = cli.command else {
        panic!("expected codex command");
    };
    assert_eq!(args.kind, AgentReportKind::Monthly);
    assert!(args.shared.json);
    assert_eq!(args.shared.timezone.as_deref(), Some("Asia/Tokyo"));
    assert_eq!(args.shared.since.as_deref(), Some("20260101"));
    assert_eq!(args.codex_speed, CodexSpeed::Standard);
}

#[test]
fn applies_schema_documented_config_file_options() {
    let config = TestConfig {
        shared_json: Some(true),
        shared_compact: Some(true),
        weekly_start: Some(WeekDay::Monday),
        blocks_active: Some(true),
        blocks_token_limit: Some("500000"),
        blocks_session_length: Some(6.0),
        statusline_visual_burn_rate: Some(VisualBurnRate::EmojiText),
        statusline_cost_source: Some(CostSource::Both),
        statusline_refresh_interval: Some(3),
        pi_path: Some("/tmp/pi-sessions"),
        open_claw_path: Some("/tmp/openclaw"),
        ..TestConfig::default()
    };

    let cli = parse_with_config(&["turbotokens", "claude", "weekly"], &config);
    let Some(Command::Weekly(args)) = cli.command else {
        panic!("expected weekly command");
    };
    assert!(args.shared.json);
    assert!(args.shared.compact);
    assert_eq!(args.start_of_week, WeekDay::Monday);

    let cli = parse_with_config(&["turbotokens", "claude", "blocks"], &config);
    let Some(Command::Blocks(args)) = cli.command else {
        panic!("expected blocks command");
    };
    assert!(args.active);
    assert_eq!(args.token_limit.as_deref(), Some("500000"));
    assert_eq!(args.session_length, 6.0);

    let cli = parse_with_config(&["turbotokens", "claude", "statusline"], &config);
    let Some(Command::Statusline(args)) = cli.command else {
        panic!("expected statusline command");
    };
    assert_eq!(args.visual_burn_rate, VisualBurnRate::EmojiText);
    assert_eq!(args.cost_source, CostSource::Both);
    assert_eq!(args.refresh_interval, 3);

    let cli = parse_with_config(&["turbotokens", "pi", "daily"], &config);
    let Some(Command::Pi(args)) = cli.command else {
        panic!("expected pi command");
    };
    assert_eq!(args.pi_path.as_deref(), Some("/tmp/pi-sessions"));

    let cli = parse_with_config(&["turbotokens", "openclaw", "daily"], &config);
    let Some(Command::OpenClaw(args)) = cli.command else {
        panic!("expected openclaw command");
    };
    assert_eq!(args.open_claw_path.as_deref(), Some("/tmp/openclaw"));
}

#[test]
fn root_help_lists_agent_namespaces_without_nested_commands() {
    let help = help_text();
    let agents = [
        "claude", "codex", "opencode", "amp", "droid", "codebuff", "hermes", "pi", "goose", "kilo",
        "copilot", "gemini", "kimi", "qwen", "grok", "openclaw", "zcode",
    ];

    for agent in agents {
        assert!(help.contains(&format!("\n  {agent} ")));
        assert!(!help.contains(&format!("\n  {agent} daily")));
    }
}

#[test]
fn root_help_lists_command_descriptions_and_follow_up_help_commands() {
    let help = help_text();

    assert!(help.contains("codex                      Show Codex token usage commands"));
    assert!(help.contains("For more info, run any command with the `--help` flag:"));
    assert!(help.contains("turbotokens codex --help"));
    assert!(!help.contains("turbotokens codex daily --help"));
}

#[test]
fn contextual_codex_help_lists_speed_choices() {
    let help = help_text_for_args(&[
        "turbotokens".to_string(),
        "codex".to_string(),
        "daily".to_string(),
        "--help".to_string(),
    ]);

    assert!(help.contains("Show Codex token usage grouped by day"));
    assert!(help.contains("USAGE:\n  turbotokens codex daily <OPTIONS>"));
    assert!(help.contains("choices: auto | standard | fast"));
}

#[test]
fn contextual_help_strips_path_like_program_name() {
    let help = help_text_for_args(&[
        "/usr/local/bin/turbotokens".to_string(),
        "codex".to_string(),
        "daily".to_string(),
    ]);

    assert!(help.contains("USAGE:\n  turbotokens codex daily <OPTIONS>"));
}

#[test]
fn contextual_help_strips_windows_program_name() {
    let help = help_text_for_args(&[
        "C:\\Tools\\turbotokens.exe".to_string(),
        "codex".to_string(),
        "daily".to_string(),
    ]);

    assert!(help.contains("USAGE:\n  turbotokens codex daily <OPTIONS>"));
}

#[test]
fn contextual_agent_help_lists_agent_subcommands() {
    let help = help_text_for_args(&["turbotokens".to_string(), "claude".to_string()]);

    assert!(help.contains("USAGE:\n  turbotokens claude <COMMANDS>"));
    assert!(help.contains("daily       Show usage report grouped by date"));
    assert!(help.contains("statusline  Display compact status line for Claude Code hooks"));
    assert!(help.contains("turbotokens claude statusline --help"));
    assert!(!help.contains("turbotokens claude daily <OPTIONS>"));
}

#[test]
fn contextual_all_agent_help_lists_color_options() {
    let help = help_text_for_args(&["turbotokens".to_string(), "daily".to_string()]);

    assert!(help.contains("--color"));
    assert!(help.contains("--no-color"));
}

#[test]
fn contextual_root_session_help_lists_id_option() {
    let help = help_text_for_args(&["turbotokens".to_string(), "session".to_string()]);

    assert!(help.contains("--id"));
}

#[test]
fn contextual_statusline_help_lists_choice_options() {
    let help = help_text_for_args(&["turbotokens".to_string(), "statusline".to_string()]);

    assert!(help.contains("choices: off | emoji | text | emoji-text"));
    assert!(help.contains("choices: auto | turbotokens | cc | both"));
}

#[test]
fn snapshots_root_and_contextual_help_text() {
    insta::assert_snapshot!("root_help", help_text());
    insta::assert_snapshot!(
        "claude_agent_help",
        help_text_for_args(&["turbotokens".to_string(), "claude".to_string()])
    );
    insta::assert_snapshot!(
        "codex_daily_help",
        help_text_for_args(&[
            "turbotokens".to_string(),
            "codex".to_string(),
            "daily".to_string(),
        ])
    );
    insta::assert_snapshot!(
        "statusline_help",
        help_text_for_args(&["turbotokens".to_string(), "statusline".to_string()])
    );
    insta::assert_snapshot!(
        "live_help",
        help_text_for_args(&["turbotokens".to_string(), "live".to_string()])
    );
    insta::assert_snapshot!(
        "stream_help",
        help_text_for_args(&["turbotokens".to_string(), "stream".to_string()])
    );
}

#[test]
fn snapshots_representative_cli_parse_shapes() {
    let cases = vec![
        json!({
            "case": "default all-agent daily",
            "cli": cli_snapshot(parse(&["turbotokens"])),
        }),
        json!({
            "case": "root daily with shared flags",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "--json",
                "--since=20260102",
                "--until",
                "20260110",
                "--mode",
                "calculate",
                "--debug",
                "--debug-samples",
                "9",
                "--order",
                "desc",
                "--breakdown",
                "--offline",
                "--no-offline",
                "--color",
                "--no-color",
                "--timezone",
                "Asia/Tokyo",
                "--jq",
                ".totals",
                "--compact",
                "--single-thread",
                "daily",
            ])),
        }),
        json!({
            "case": "claude weekly monday",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "claude",
                "weekly",
                "--start-of-week",
                "monday",
            ])),
        }),
        json!({
            "case": "claude daily project instances",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "claude",
                "daily",
                "--instances",
                "--project",
                "repo",
                "--project-aliases",
                "repo=Repository",
            ])),
        }),
        json!({
            "case": "codex monthly fast",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "codex",
                "monthly",
                "--speed=fast",
            ])),
        }),
        json!({
            "case": "opencode weekly",
            "cli": cli_snapshot(parse(&["turbotokens", "opencode", "weekly", "--json"])),
        }),
        json!({
            "case": "antigravity session",
            "cli": cli_snapshot(parse(&["turbotokens", "antigravity", "session", "--json"])),
        }),
        json!({
            "case": "live with interval",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "live",
                "--json",
                "--interval",
                "250",
                "--offline",
            ])),
        }),
        json!({
            "case": "pi session path",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "pi",
                "session",
                "--pi-path",
                "/tmp/pi-sessions",
            ])),
        }),
        json!({
            "case": "openclaw session path",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "openclaw",
                "session",
                "--open-claw-path=/tmp/openclaw",
            ])),
        }),
        json!({
            "case": "blocks active recent",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "blocks",
                "--active",
                "--recent",
                "--token-limit",
                "max",
                "--session-length=6.5",
            ])),
        }),
        json!({
            "case": "statusline thresholds",
            "cli": cli_snapshot(parse(&[
                "turbotokens",
                "statusline",
                "--no-offline",
                "--visual-burn-rate",
                "emoji-text",
                "--cost-source",
                "both",
                "--no-cache",
                "--refresh-interval",
                "3",
                "--context-low-threshold",
                "45",
                "--context-medium-threshold",
                "75",
                "--debug",
            ])),
        }),
    ];

    insta::assert_json_snapshot!(cases);
}

#[test]
fn snapshots_cli_parse_error_guidance() {
    let cases = vec![
        json!({
            "args": ["turbotokens", "--daily"],
            "error": parse_error(&["turbotokens", "--daily"]),
        }),
        json!({
            "args": ["turbotokens", "daily", "--agent", "codex"],
            "error": parse_error(&["turbotokens", "daily", "--agent", "codex"]),
        }),
        json!({
            "args": ["turbotokens", "codex", "blocks"],
            "error": parse_error(&["turbotokens", "codex", "blocks"]),
        }),
        json!({
            "args": ["turbotokens", "--mode", "bad"],
            "error": parse_error(&["turbotokens", "--mode", "bad"]),
        }),
        json!({
            "args": ["turbotokens", "blocks", "--session-length", "abc"],
            "error": parse_error(&["turbotokens", "blocks", "--session-length", "abc"]),
        }),
        json!({
            "args": ["turbotokens", "statusline", "--visual-burn-rate", "loud"],
            "error": parse_error(&[
                "turbotokens",
                "statusline",
                "--visual-burn-rate",
                "loud",
            ]),
        }),
        json!({
            "args": ["turbotokens", "pi", "weekly"],
            "error": parse_error(&["turbotokens", "pi", "weekly"]),
        }),
    ];

    insta::assert_json_snapshot!(cases);
}

#[test]
fn parses_claude_daily_options() {
    let cli = parse(&[
        "turbotokens",
        "claude",
        "daily",
        "--json",
        "--mode",
        "display",
        "--instances",
        "--project",
        "repo",
    ]);
    let Some(Command::Daily(args)) = cli.command else {
        panic!("expected daily command");
    };
    assert!(args.shared.json);
    assert_eq!(args.shared.mode, CostMode::Display);
    assert!(args.instances);
    assert_eq!(args.project.as_deref(), Some("repo"));
}

#[test]
fn rejects_removed_locale_option() {
    let result = Cli::parse_from(
        ["turbotokens", "--locale", "en-CA"]
            .into_iter()
            .map(OsString::from),
    );
    assert!(result.is_err());
}

#[test]
fn parses_blocks_defaults_and_values() {
    let cli = parse(&[
        "turbotokens",
        "blocks",
        "-a",
        "--token-limit=max",
        "--session-length",
        "6",
    ]);
    let Some(Command::Blocks(args)) = cli.command else {
        panic!("expected blocks command");
    };
    assert!(args.active);
    assert_eq!(args.token_limit.as_deref(), Some("max"));
    assert_eq!(args.session_length, 6.0);
}

#[test]
fn parses_claude_blocks_short_active_option() {
    let cli = parse(&["turbotokens", "claude", "blocks", "-a"]);
    let Some(Command::Blocks(args)) = cli.command else {
        panic!("expected blocks command");
    };
    assert!(args.active);
}

#[test]
fn parses_statusline_options() {
    let cli = parse(&[
        "turbotokens",
        "statusline",
        "--no-cache",
        "--timezone",
        "Asia/Tokyo",
        "--visual-burn-rate",
        "emoji-text",
        "--cost-source",
        "both",
    ]);
    let Some(Command::Statusline(args)) = cli.command else {
        panic!("expected statusline command");
    };
    assert!(args.offline);
    assert!(args.no_cache);
    assert_eq!(args.timezone.as_deref(), Some("Asia/Tokyo"));
    assert_eq!(args.visual_burn_rate, VisualBurnRate::EmojiText);
    assert_eq!(args.cost_source, CostSource::Both);
}

#[test]
fn parses_codex_default_daily_options() {
    let cli = parse(&["turbotokens", "codex", "--json", "--since", "20260102"]);
    let Some(Command::Codex(args)) = cli.command else {
        panic!("expected codex command");
    };
    assert_eq!(args.kind, AgentReportKind::Daily);
    assert!(args.shared.json);
    assert_eq!(args.shared.since.as_deref(), Some("20260102"));
}

#[test]
fn parses_codex_speed_option() {
    let cli = parse(&["turbotokens", "codex", "daily", "--speed", "fast"]);
    let Some(Command::Codex(args)) = cli.command else {
        panic!("expected codex command");
    };
    assert_eq!(args.codex_speed, CodexSpeed::Fast);
}

#[test]
fn parses_legacy_colon_agent_commands() {
    let cli = parse(&["turbotokens", "codex:monthly", "--json"]);
    let Some(Command::Codex(args)) = cli.command else {
        panic!("expected codex command");
    };
    assert_eq!(args.kind, AgentReportKind::Monthly);
    assert!(args.shared.json);
}

#[test]
fn rejects_report_flag_aliases_with_guidance() {
    let error = parse_error(&["turbotokens", "--daily"]);
    assert_eq!(
        error,
        "Report flags like --daily are not supported. Use \"turbotokens daily\" instead."
    );
}

#[test]
fn rejects_agent_filter_options_with_guidance() {
    let error = parse_error(&["turbotokens", "daily", "--agent", "codex"]);
    assert_eq!(
        error,
        "Agent filters like --agent are not supported. Use \"turbotokens <agent> <report>\", for example \"turbotokens codex daily\"."
    );
}

#[test]
fn rejects_unsupported_agent_reports_with_guidance() {
    let error = parse_error(&["turbotokens", "codex", "blocks"]);
    assert_eq!(
        error,
        "The \"blocks\" report is only available for Claude Code usage.\nUse \"turbotokens codex daily\" for Codex usage reports."
    );
}

#[test]
fn parses_claude_namespace_session_options() {
    let cli = parse(&["turbotokens", "claude", "session", "--json", "--id", "abc"]);
    let Some(Command::Session(args)) = cli.command else {
        panic!("expected claude session command");
    };
    assert!(args.shared.json);
    assert_eq!(args.id.as_deref(), Some("abc"));
}

#[test]
fn parses_top_level_session_id_lookup() {
    let cli = parse(&["turbotokens", "session", "--json", "--id", "abc"]);
    let Some(Command::Session(args)) = cli.command else {
        panic!("expected session command");
    };
    assert!(args.shared.json);
    assert_eq!(args.id.as_deref(), Some("abc"));
}

#[test]
fn parses_opencode_weekly_options() {
    let cli = parse(&["turbotokens", "opencode", "weekly", "--json"]);
    let Some(Command::OpenCode(args)) = cli.command else {
        panic!("expected opencode command");
    };
    assert_eq!(args.kind, AgentReportKind::Weekly);
    assert!(args.shared.json);
}

#[test]
fn parses_amp_session_options() {
    let cli = parse(&["turbotokens", "amp", "session", "--json"]);
    let Some(Command::Amp(args)) = cli.command else {
        panic!("expected amp command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_droid_session_options() {
    let cli = parse(&["turbotokens", "droid", "session", "--json"]);
    let Some(Command::Droid(args)) = cli.command else {
        panic!("expected droid command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_codebuff_session_options() {
    let cli = parse(&["turbotokens", "codebuff", "session", "--json"]);
    let Some(Command::Codebuff(args)) = cli.command else {
        panic!("expected codebuff command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_qwen_session_options() {
    let cli = parse(&["turbotokens", "qwen", "session", "--json"]);
    let Some(Command::Qwen(args)) = cli.command else {
        panic!("expected qwen command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_pi_session_options() {
    let cli = parse(&[
        "turbotokens",
        "pi",
        "session",
        "--json",
        "--pi-path",
        "/tmp/pi-sessions",
    ]);
    let Some(Command::Pi(args)) = cli.command else {
        panic!("expected pi command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
    assert_eq!(args.pi_path.as_deref(), Some("/tmp/pi-sessions"));
}

#[test]
fn parses_kilo_session_options() {
    let cli = parse(&["turbotokens", "kilo", "session", "--json"]);
    let Some(Command::Kilo(args)) = cli.command else {
        panic!("expected kilo command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_goose_session_options() {
    let cli = parse(&["turbotokens", "goose", "session", "--json"]);
    let Some(Command::Goose(args)) = cli.command else {
        panic!("expected goose command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_copilot_session_options() {
    let cli = parse(&["turbotokens", "copilot", "session", "--json"]);
    let Some(Command::Copilot(args)) = cli.command else {
        panic!("expected copilot command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_gemini_session_options() {
    let cli = parse(&["turbotokens", "gemini", "session", "--json"]);
    let Some(Command::Gemini(args)) = cli.command else {
        panic!("expected gemini command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_antigravity_weekly_and_session_options() {
    let cli = parse(&["turbotokens", "antigravity", "weekly", "--json"]);
    let Some(Command::Antigravity(args)) = cli.command else {
        panic!("expected antigravity command");
    };
    assert_eq!(args.kind, AgentReportKind::Weekly);
    assert!(args.shared.json);

    let cli = parse(&["turbotokens", "antigravity", "session", "--json"]);
    let Some(Command::Antigravity(args)) = cli.command else {
        panic!("expected antigravity command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_kimi_session_options() {
    let cli = parse(&["turbotokens", "kimi", "session", "--json"]);
    let Some(Command::Kimi(args)) = cli.command else {
        panic!("expected kimi command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_grok_session_options() {
    let cli = parse(&["turbotokens", "grok", "session", "--json"]);
    let Some(Command::Grok(args)) = cli.command else {
        panic!("expected grok command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn parses_openclaw_session_options() {
    let cli = parse(&[
        "turbotokens",
        "openclaw",
        "session",
        "--json",
        "--open-claw-path",
        "/tmp/openclaw",
    ]);
    let Some(Command::OpenClaw(args)) = cli.command else {
        panic!("expected openclaw command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
    assert_eq!(args.open_claw_path.as_deref(), Some("/tmp/openclaw"));
}

#[test]
fn parses_zcode_weekly_and_session_options() {
    let cli = parse(&["turbotokens", "zcode", "weekly", "--json"]);
    let Some(Command::ZCode(args)) = cli.command else {
        panic!("expected zcode command");
    };
    assert_eq!(args.kind, AgentReportKind::Weekly);
    assert!(args.shared.json);

    let cli = parse(&["turbotokens", "zcode", "session", "--json"]);
    let Some(Command::ZCode(args)) = cli.command else {
        panic!("expected zcode command");
    };
    assert_eq!(args.kind, AgentReportKind::Session);
    assert!(args.shared.json);
}

#[test]
fn named_pi_store_validation_does_not_break_statusline() {
    let fixture = fs_fixture!({
        "turbotokens.json": r#"{ "pi": { "stores": [{ "name": "codex", "path": "/tmp/omp" }] } }"#,
    });
    let args = vec![
        "statusline".to_string(),
        "--config".to_string(),
        fixture
            .path("turbotokens.json")
            .to_string_lossy()
            .into_owned(),
    ];
    let config = turbotokens_config::ConfigContext::from_args(&args);

    let parsed = Cli::parse_from_with_config(
        std::iter::once(std::ffi::OsString::from("turbotokens")).chain(
            args.iter()
                .map(|arg| std::ffi::OsString::from(arg.as_str())),
        ),
        &config,
        turbotokens_core::DEFAULT_SESSION_DURATION_HOURS,
        env!("CARGO_PKG_VERSION"),
    );

    assert!(parsed.is_ok());
}

#[test]
fn named_pi_store_validation_does_not_break_agent_commands() {
    let fixture = fs_fixture!({
        "turbotokens.json": r#"{ "pi": { "stores": [{ "name": "codex", "path": "/tmp/omp" }] } }"#,
    });
    let args = vec![
        "codex".to_string(),
        "daily".to_string(),
        "--config".to_string(),
        fixture
            .path("turbotokens.json")
            .to_string_lossy()
            .into_owned(),
    ];
    let config = turbotokens_config::ConfigContext::from_args(&args);

    let parsed = Cli::parse_from_with_config(
        std::iter::once(std::ffi::OsString::from("turbotokens")).chain(
            args.iter()
                .map(|arg| std::ffi::OsString::from(arg.as_str())),
        ),
        &config,
        turbotokens_core::DEFAULT_SESSION_DURATION_HOURS,
        env!("CARGO_PKG_VERSION"),
    );

    assert!(parsed.is_ok());
}

#[test]
fn reports_named_pi_store_validation_through_cli_config_error_path() {
    let fixture = fs_fixture!({
        "turbotokens.json": r#"{ "pi": { "stores": [{ "name": "codex", "path": "/tmp/omp" }] } }"#,
    });
    let args = vec![
        "daily".to_string(),
        "--config".to_string(),
        fixture
            .path("turbotokens.json")
            .to_string_lossy()
            .into_owned(),
    ];
    let config = turbotokens_config::ConfigContext::from_args(&args);

    let result = Cli::parse_from_with_config(
        std::iter::once(std::ffi::OsString::from("turbotokens")).chain(
            args.iter()
                .map(|arg| std::ffi::OsString::from(arg.as_str())),
        ),
        &config,
        turbotokens_core::DEFAULT_SESSION_DURATION_HOURS,
        env!("CARGO_PKG_VERSION"),
    );
    let Err(error) = result else {
        panic!("expected config error");
    };

    assert_eq!(
        error,
        "Invalid turbotokens config: pi.stores name 'codex' collides with a built-in agent"
    );
}

#[test]
fn parses_limits_command_scopes() {
    let cli = parse(&["turbotokens", "limits", "--json"]);
    let Some(Command::Limits(args)) = cli.command else {
        panic!("expected limits command");
    };
    assert_eq!(args.scope, LimitsScope::All);
    assert!(args.shared.json);

    let cli = parse(&["turbotokens", "claude", "limits", "--no-color"]);
    let Some(Command::Limits(args)) = cli.command else {
        panic!("expected limits command");
    };
    assert_eq!(args.scope, LimitsScope::Claude);
    assert!(args.shared.no_color);

    let cli = parse(&["turbotokens", "codex", "limits", "-z", "Asia/Tokyo"]);
    let Some(Command::Limits(args)) = cli.command else {
        panic!("expected limits command");
    };
    assert_eq!(args.scope, LimitsScope::Codex);
    assert_eq!(args.shared.timezone.as_deref(), Some("Asia/Tokyo"));
}

#[test]
fn rejects_limits_command_for_agents_without_plan_limits() {
    assert_eq!(
        parse_error(&["turbotokens", "gemini", "limits"]),
        "The \"limits\" report is not available for Gemini CLI usage.\nUse \"turbotokens gemini daily\" for Gemini CLI usage reports."
    );
}

#[test]
fn rejects_last_periods_on_limits() {
    assert_eq!(
        parse_error(&["turbotokens", "limits", "--last", "1"]),
        "The --last option is only available for the daily, weekly, and monthly reports."
    );
}

#[test]
fn parses_import_command_with_file_and_shared_flags() {
    let cli = parse(&["turbotokens", "import", "/tmp/export.json"]);
    let Some(Command::Import(args)) = cli.command else {
        panic!("expected import command");
    };
    assert_eq!(args.file, std::path::PathBuf::from("/tmp/export.json"));

    let cli = parse(&[
        "turbotokens",
        "import",
        "export.json",
        "--json",
        "--breakdown",
        "--since",
        "2026-01-01",
    ]);
    let Some(Command::Import(args)) = cli.command else {
        panic!("expected import command");
    };
    assert_eq!(args.file, std::path::PathBuf::from("export.json"));
    assert!(args.shared.json);
    assert!(args.shared.breakdown);
    assert_eq!(args.shared.since.as_deref(), Some("20260101"));
}

#[test]
fn import_command_requires_a_file() {
    assert_eq!(
        parse_error(&["turbotokens", "import"]),
        "Usage: turbotokens import <FILE> (a ccusage JSON export, e.g. from `ccusage daily --json`)"
    );
    assert_eq!(
        parse_error(&["turbotokens", "import", "a.json", "b.json"]),
        "Unexpected argument 'b.json'"
    );
}

#[test]
fn rejects_last_periods_on_import() {
    assert_eq!(
        parse_error(&["turbotokens", "import", "export.json", "--last", "1"]),
        "The --last option is only available for the daily, weekly, and monthly reports."
    );
}

#[test]
fn parses_heatmap_command_with_cost_svg_and_window() {
    let cli = parse(&["turbotokens", "heatmap"]);
    let Some(Command::Heatmap(args)) = cli.command else {
        panic!("expected heatmap command");
    };
    assert!(!args.cost);
    assert_eq!(args.svg, None);

    let cli = parse(&[
        "turbotokens",
        "heatmap",
        "--cost",
        "--svg",
        "heat.svg",
        "--since",
        "2026-01-01",
        "--json",
    ]);
    let Some(Command::Heatmap(args)) = cli.command else {
        panic!("expected heatmap command");
    };
    assert!(args.cost);
    assert_eq!(args.svg.as_deref(), Some("heat.svg"));
    assert_eq!(args.shared.since.as_deref(), Some("20260101"));
    assert!(args.shared.json);

    assert!(parse_error(&["turbotokens", "heatmap", "--bogus"]).contains("Unknown heatmap option"));
}

#[test]
fn parses_wrapped_command_with_year_and_svg() {
    let cli = parse(&["turbotokens", "wrapped"]);
    let Some(Command::Wrapped(args)) = cli.command else {
        panic!("expected wrapped command");
    };
    assert_eq!(args.year, None);

    let cli = parse(&["turbotokens", "wrapped", "--year", "2025", "--svg=card.svg"]);
    let Some(Command::Wrapped(args)) = cli.command else {
        panic!("expected wrapped command");
    };
    assert_eq!(args.year, Some(2025));
    assert_eq!(args.svg.as_deref(), Some("card.svg"));

    assert!(parse_error(&["turbotokens", "wrapped", "--year", "20"]).contains("--year"));
    assert!(parse_error(&["turbotokens", "wrapped", "--year", "abc"]).contains("--year"));
    assert!(parse_error(&["turbotokens", "wrapped", "--bogus"]).contains("Unknown wrapped option"));
}

#[test]
fn rejects_last_periods_on_visual_commands() {
    for command in ["heatmap", "wrapped"] {
        assert_eq!(
            parse_error(&["turbotokens", command, "--last", "2"]),
            "The --last option is only available for the daily, weekly, and monthly reports."
        );
    }
}
