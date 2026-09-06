//! `turbotokens import` — render a ccusage aggregate JSON export through the
//! native turbotokens reports.
//!
//! Migration demo path: a ccusage user saves `ccusage daily --json` (or
//! monthly/weekly/session) to a file and runs `turbotokens import <FILE>` to
//! see the same data through turbotokens' tables without re-reading raw logs.
//!
//! Two export shapes are accepted:
//!
//! - the classic per-agent shape (`ccusage claude daily --json`): rows keyed by
//!   `date` / `month` / `week` / `sessionId`, sessions under a `sessions` array
//! - the unified all-agents shape (ccusage 20 `ccusage daily --json`): rows
//!   keyed by `period` with an extra `agent` field, sessions under a `session`
//!   array
//!
//! Both carry `modelBreakdowns` per row and a `totals` object; totals are
//! recomputed from the rows so `--json` output is always internally
//! consistent.

use std::{fs, path::Path};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Context as _, ModelBreakdown, Result, UsageSummary, cli::ImportArgs, cli_error,
    filter_and_sort_summaries, print_json_or_jq, print_usage_table, session_summary_json,
    sort_summaries, summary_json, totals_json, wants_json,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImportKind {
    Daily,
    Weekly,
    Monthly,
    Session,
}

impl ImportKind {
    /// Array key(s) holding this report's rows, most common spelling first.
    fn array_names(self) -> &'static [&'static str] {
        match self {
            Self::Daily => &["daily"],
            Self::Weekly => &["weekly"],
            Self::Monthly => &["monthly"],
            Self::Session => &["sessions", "session"],
        }
    }

    /// The row field that keys the report; unified ccusage 20 exports use
    /// `period` for every report instead.
    fn key_field(self) -> &'static str {
        match self {
            Self::Daily => "date",
            Self::Weekly => "week",
            Self::Monthly => "month",
            Self::Session => "sessionId",
        }
    }

    fn json_key(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Session => "sessions",
        }
    }

    fn table_title(self) -> &'static str {
        match self {
            Self::Daily => "Claude Code Token Usage Report - Daily",
            Self::Weekly => "Claude Code Token Usage Report - Weekly",
            Self::Monthly => "Claude Code Token Usage Report - Monthly",
            Self::Session => "Claude Code Token Usage Report - By Session",
        }
    }

    fn first_column(self) -> &'static str {
        match self {
            Self::Daily => "Date",
            Self::Weekly => "Week",
            Self::Monthly => "Month",
            Self::Session => "Session",
        }
    }
}

/// One row of a ccusage export. Only the shared token/cost fields are
/// required; the report key and session metadata differ per report and the
/// unified shape folds them all into `period`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRow {
    date: Option<String>,
    week: Option<String>,
    month: Option<String>,
    session_id: Option<String>,
    period: Option<String>,
    project_path: Option<String>,
    last_activity: Option<String>,
    first_activity: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    /// ccusage precomputes this per row; it can exceed the sum of the four
    /// counted fields (e.g. reasoning tokens the export only folds into the
    /// total). The residual is kept as `extra_total_tokens` so recomputed
    /// totals still match the export.
    total_tokens: Option<u64>,
    total_cost: f64,
    #[serde(default)]
    models_used: Vec<String>,
    #[serde(default)]
    model_breakdowns: Vec<ModelBreakdown>,
}

pub(super) fn run(args: &ImportArgs) -> Result<()> {
    let content = fs::read_to_string(&args.file)
        .context(format!("could not read {}", args.file.display()))?;
    let (kind, mut rows) = parse_export(&content, &args.file)?;

    match kind {
        ImportKind::Daily => {
            filter_and_sort_summaries(&mut rows, &args.shared, |row| {
                row.date.as_deref().unwrap_or_default()
            });
        }
        ImportKind::Weekly => {
            filter_and_sort_summaries(&mut rows, &args.shared, |row| {
                row.week.as_deref().unwrap_or_default()
            });
        }
        // Months are coarser than the day-granular --since/--until window, so
        // monthly rows are only sorted.
        ImportKind::Monthly => {
            sort_summaries(&mut rows, &args.shared.order, |row| {
                row.month.as_deref().unwrap_or_default()
            });
        }
        // Sessions order by cost like the native session report; --since/--until
        // filter on the last-activity date.
        ImportKind::Session => {
            rows.retain(|row| {
                crate::date_within_range(
                    row.last_activity
                        .as_deref()
                        .and_then(|activity| activity.get(..10))
                        .unwrap_or_default(),
                    args.shared.since.as_deref(),
                    args.shared.until.as_deref(),
                )
            });
            rows.sort_by(|a, b| b.total_cost.total_cmp(&a.total_cost));
        }
    }

    if wants_json(&args.shared) {
        let rows_json = match kind {
            ImportKind::Session => rows.iter().map(session_summary_json).collect::<Vec<_>>(),
            _ => rows.iter().map(summary_json).collect::<Vec<_>>(),
        };
        let mut output = serde_json::Map::new();
        output.insert(kind.json_key().to_string(), json!(rows_json));
        output.insert("totals".to_string(), totals_json(&rows));
        print_json_or_jq(
            Value::Object(output),
            args.shared.jq.as_deref(),
            args.shared.no_cost,
        )?;
        return Ok(());
    }

    print_usage_table(
        kind.table_title(),
        kind.first_column(),
        &rows,
        &args.shared,
        false,
        None,
    )?;
    Ok(())
}

fn parse_export(content: &str, path: &Path) -> Result<(ImportKind, Vec<UsageSummary>)> {
    let display = path.display();
    let root: Value = serde_json::from_str(content).map_err(|error| {
        cli_error(format!(
            "{display}: not valid JSON ({error}); expected a ccusage JSON export such as `ccusage daily --json` output"
        ))
    })?;
    let Value::Object(root) = root else {
        return Err(shape_error(display));
    };
    for kind in [
        ImportKind::Daily,
        ImportKind::Weekly,
        ImportKind::Monthly,
        ImportKind::Session,
    ] {
        for name in kind.array_names() {
            if let Some(value) = root.get(*name) {
                let Value::Array(rows) = value else {
                    return Err(cli_error(format!(
                        "{display}: \"{name}\" is not an array; {}",
                        shape_hint()
                    )));
                };
                let rows = parse_rows(kind, name, rows, path)?;
                return Ok((kind, rows));
            }
        }
    }
    Err(shape_error(display))
}

fn shape_hint() -> &'static str {
    "expected a ccusage export with a \"daily\", \"weekly\", \"monthly\", or \"sessions\" array plus \"totals\" (the shape of `ccusage daily --json` / `ccusage monthly --json` / `ccusage session --json`)"
}

fn shape_error(display: std::path::Display<'_>) -> crate::CliError {
    cli_error(format!(
        "{display}: this doesn't look like a ccusage export — {}",
        shape_hint()
    ))
}

fn parse_rows(
    kind: ImportKind,
    array_name: &str,
    rows: &[Value],
    path: &Path,
) -> Result<Vec<UsageSummary>> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| parse_row(kind, array_name, index, row, path))
        .collect()
}

fn parse_row(
    kind: ImportKind,
    array_name: &str,
    index: usize,
    row: &Value,
    path: &Path,
) -> Result<UsageSummary> {
    let display = path.display();
    let parsed: ExportRow = serde_json::from_value(row.clone()).map_err(|error| {
        cli_error(format!(
            "{display}: row {} of \"{array_name}\" is not a ccusage export row ({error})",
            index + 1
        ))
    })?;
    let key = match kind {
        ImportKind::Daily => parsed.date.or(parsed.period),
        ImportKind::Weekly => parsed.week.or(parsed.period),
        ImportKind::Monthly => parsed.month.or(parsed.period),
        ImportKind::Session => parsed.session_id.or(parsed.period),
    };
    let Some(key) = key.filter(|key| !key.is_empty()) else {
        return Err(cli_error(format!(
            "{display}: row {} of \"{array_name}\" has no \"{}\" (or \"period\") key",
            index + 1,
            kind.key_field()
        )));
    };
    let mut summary = UsageSummary {
        date: (kind == ImportKind::Daily).then(|| key.clone()),
        month: (kind == ImportKind::Monthly).then(|| key.clone()),
        week: (kind == ImportKind::Weekly).then(|| key.clone()),
        session_id: (kind == ImportKind::Session).then_some(key),
        project_path: parsed.project_path,
        last_activity: parsed.last_activity,
        first_activity: parsed.first_activity,
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        cache_creation_tokens: parsed.cache_creation_tokens,
        cache_read_tokens: parsed.cache_read_tokens,
        extra_total_tokens: parsed.total_tokens.map_or(0, |total| {
            total.saturating_sub(
                parsed
                    .input_tokens
                    .saturating_add(parsed.output_tokens)
                    .saturating_add(parsed.cache_creation_tokens)
                    .saturating_add(parsed.cache_read_tokens),
            )
        }),
        total_cost: parsed.total_cost,
        credits: None,
        message_count: None,
        models_used: parsed.models_used,
        model_breakdowns: parsed.model_breakdowns,
        project: None,
        versions: None,
    };
    // Rows without a modelsUsed list still break down by model, so derive the
    // list from the breakdowns to keep the table's Models column populated.
    if summary.models_used.is_empty() {
        summary.models_used = summary
            .model_breakdowns
            .iter()
            .map(|breakdown| breakdown.model_name.clone())
            .collect();
    }
    // Match the native aggregation's cost-descending breakdown order.
    summary
        .model_breakdowns
        .sort_by(|a, b| b.cost.total_cmp(&a.cost));
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cli::{SharedArgs, SortOrder};

    const CLASSIC_DAILY: &str = r#"{
        "daily": [
            {
                "date": "2026-04-16",
                "inputTokens": 18,
                "outputTokens": 2214,
                "cacheCreationTokens": 30947,
                "cacheReadTokens": 320932,
                "totalTokens": 354111,
                "totalCost": 0.525376,
                "modelsUsed": ["claude-opus-4-7"],
                "modelBreakdowns": [
                    {
                        "modelName": "claude-opus-4-7",
                        "inputTokens": 18,
                        "outputTokens": 2214,
                        "cacheCreationTokens": 30947,
                        "cacheReadTokens": 320932,
                        "cost": 0.525376
                    }
                ]
            },
            {
                "date": "2026-04-15",
                "inputTokens": 100,
                "outputTokens": 50,
                "cacheCreationTokens": 10,
                "cacheReadTokens": 5,
                "totalTokens": 165,
                "totalCost": 0.01,
                "modelsUsed": ["claude-sonnet-4-20250514"],
                "modelBreakdowns": [
                    {
                        "modelName": "claude-sonnet-4-20250514",
                        "inputTokens": 100,
                        "outputTokens": 50,
                        "cacheCreationTokens": 10,
                        "cacheReadTokens": 5,
                        "cost": 0.01
                    }
                ]
            }
        ],
        "totals": {
            "inputTokens": 118,
            "outputTokens": 2264,
            "cacheCreationTokens": 30957,
            "cacheReadTokens": 320937,
            "totalTokens": 354276,
            "totalCost": 0.535376
        }
    }"#;

    fn path() -> &'static Path {
        Path::new("export.json")
    }

    #[test]
    fn parses_classic_daily_export_into_usage_summaries() {
        let (kind, rows) = parse_export(CLASSIC_DAILY, path()).unwrap();

        assert_eq!(kind, ImportKind::Daily);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date.as_deref(), Some("2026-04-16"));
        assert_eq!(rows[0].input_tokens, 18);
        assert_eq!(rows[0].output_tokens, 2214);
        assert_eq!(rows[0].cache_creation_tokens, 30_947);
        assert_eq!(rows[0].cache_read_tokens, 320_932);
        assert_eq!(rows[0].total_cost, 0.525_376);
        assert_eq!(rows[0].models_used, vec!["claude-opus-4-7".to_string()]);
        assert_eq!(rows[0].model_breakdowns.len(), 1);
        assert_eq!(rows[0].model_breakdowns[0].model_name, "claude-opus-4-7");
    }

    #[test]
    fn parses_unified_period_shape_with_agent_and_metadata() {
        let export = r#"{
            "daily": [
                {
                    "agent": "all",
                    "metadata": {"agents": ["codex"]},
                    "period": "2025-10-11",
                    "inputTokens": 121915,
                    "outputTokens": 27798,
                    "cacheCreationTokens": 0,
                    "cacheReadTokens": 3445632,
                    "totalTokens": 3595345,
                    "totalCost": 0.86107775,
                    "modelsUsed": ["gpt-5-codex"],
                    "modelBreakdowns": [
                        {
                            "modelName": "gpt-5-codex",
                            "inputTokens": 121915,
                            "outputTokens": 27798,
                            "cacheCreationTokens": 0,
                            "cacheReadTokens": 3445632,
                            "cost": 0.86107775
                        }
                    ]
                }
            ],
            "totals": {"inputTokens": 121915, "outputTokens": 27798, "cacheCreationTokens": 0, "cacheReadTokens": 3445632, "totalTokens": 3595345, "totalCost": 0.86107775}
        }"#;

        let (kind, rows) = parse_export(export, path()).unwrap();

        assert_eq!(kind, ImportKind::Daily);
        assert_eq!(rows[0].date.as_deref(), Some("2025-10-11"));
        assert_eq!(rows[0].cache_read_tokens, 3_445_632);
    }

    #[test]
    fn parses_classic_monthly_weekly_and_session_exports() {
        let monthly = r#"{"monthly": [{"month": "2026-04", "inputTokens": 1, "outputTokens": 2, "cacheCreationTokens": 3, "cacheReadTokens": 4, "totalTokens": 10, "totalCost": 0.5, "modelsUsed": [], "modelBreakdowns": []}], "totals": {}}"#;
        let weekly = r#"{"weekly": [{"week": "2026-04-12", "inputTokens": 1, "outputTokens": 2, "cacheCreationTokens": 3, "cacheReadTokens": 4, "totalTokens": 10, "totalCost": 0.5, "modelsUsed": [], "modelBreakdowns": []}], "totals": {}}"#;
        let session = r#"{"sessions": [{"sessionId": "abc-123", "projectPath": "/Users/me/repo", "lastActivity": "2026-08-22T04:18:50.084Z", "firstActivity": "2026-06-14T04:19:20.528Z", "inputTokens": 1, "outputTokens": 2, "cacheCreationTokens": 3, "cacheReadTokens": 4, "totalTokens": 10, "totalCost": 0.5, "modelsUsed": ["claude-opus-4-7"], "modelBreakdowns": []}], "totals": {}}"#;

        let (kind, rows) = parse_export(monthly, path()).unwrap();
        assert_eq!(kind, ImportKind::Monthly);
        assert_eq!(rows[0].month.as_deref(), Some("2026-04"));

        let (kind, rows) = parse_export(weekly, path()).unwrap();
        assert_eq!(kind, ImportKind::Weekly);
        assert_eq!(rows[0].week.as_deref(), Some("2026-04-12"));

        let (kind, rows) = parse_export(session, path()).unwrap();
        assert_eq!(kind, ImportKind::Session);
        assert_eq!(rows[0].session_id.as_deref(), Some("abc-123"));
        assert_eq!(rows[0].project_path.as_deref(), Some("/Users/me/repo"));
        assert_eq!(
            rows[0].last_activity.as_deref(),
            Some("2026-08-22T04:18:50.084Z")
        );
    }

    #[test]
    fn parses_unified_session_array_singular_key() {
        let export = r#"{"session": [{"agent": "droid", "period": "0038a208-74ad-4724-af9c-4d6bc0d0b8ab", "inputTokens": 45017, "outputTokens": 14527, "cacheCreationTokens": 0, "cacheReadTokens": 1366016, "totalTokens": 1431229, "totalCost": 0.60057655, "modelsUsed": ["gpt-5-3-codex"], "modelBreakdowns": []}], "totals": {}}"#;

        let (kind, rows) = parse_export(export, path()).unwrap();

        assert_eq!(kind, ImportKind::Session);
        assert_eq!(
            rows[0].session_id.as_deref(),
            Some("0038a208-74ad-4724-af9c-4d6bc0d0b8ab")
        );
    }

    #[test]
    fn derives_models_used_from_breakdowns_when_missing() {
        let export = r#"{"daily": [{"date": "2026-04-16", "inputTokens": 1, "outputTokens": 2, "cacheCreationTokens": 3, "cacheReadTokens": 4, "totalCost": 0.5, "modelBreakdowns": [{"modelName": "gpt-5", "inputTokens": 1, "outputTokens": 2, "cacheCreationTokens": 3, "cacheReadTokens": 4, "cost": 0.5}]}], "totals": {}}"#;

        let (_, rows) = parse_export(export, path()).unwrap();

        assert_eq!(rows[0].models_used, vec!["gpt-5".to_string()]);
    }

    #[test]
    fn rejects_non_json_with_pointer_to_ccusage_export() {
        let error = parse_export("not json", path()).unwrap_err();

        assert!(error.to_string().contains("not valid JSON"));
        assert!(error.to_string().contains("ccusage daily --json"));
    }

    #[test]
    fn rejects_unrecognized_shape_with_clear_error() {
        for export in ["{}", r#"{"rows": []}"#, "[]", r#"{"daily": {}}"#] {
            let error = parse_export(export, path()).unwrap_err();
            assert!(
                error.to_string().contains("ccusage export"),
                "export {export}: {error}"
            );
        }
        let error = parse_export("{}", path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("doesn't look like a ccusage export")
        );
        assert!(error.to_string().contains("\"daily\""));
        assert!(error.to_string().contains("totals"));
    }

    #[test]
    fn rejects_row_missing_token_fields_with_row_index() {
        let export = r#"{"daily": [{"date": "2026-04-16", "outputTokens": 2}], "totals": {}}"#;

        let error = parse_export(export, path()).unwrap_err();

        assert!(error.to_string().contains("row 1 of \"daily\""));
        assert!(error.to_string().contains("inputTokens"));
    }

    #[test]
    fn rejects_row_without_period_key() {
        let export = r#"{"daily": [{"inputTokens": 1, "outputTokens": 2, "cacheCreationTokens": 3, "cacheReadTokens": 4, "totalCost": 0.5}], "totals": {}}"#;

        let error = parse_export(export, path()).unwrap_err();

        assert!(error.to_string().contains("row 1 of \"daily\""));
        assert!(error.to_string().contains("\"date\""));
    }

    #[test]
    fn json_output_matches_native_daily_shape() {
        let (kind, mut rows) = parse_export(CLASSIC_DAILY, path()).unwrap();
        let shared = SharedArgs::default();
        filter_and_sort_summaries(&mut rows, &shared, |row| {
            row.date.as_deref().unwrap_or_default()
        });
        assert_eq!(kind.json_key(), "daily");

        let output = json!({
            "daily": rows.iter().map(summary_json).collect::<Vec<_>>(),
            "totals": totals_json(&rows),
        });

        // Ascending order puts 2026-04-15 first, and totals are recomputed
        // from the rows rather than trusted from the export.
        assert_eq!(output["daily"][0]["date"], json!("2026-04-15"));
        assert_eq!(output["daily"][1]["date"], json!("2026-04-16"));
        assert_eq!(output["totals"]["inputTokens"], json!(118));
        assert_eq!(output["totals"]["totalTokens"], json!(354276));
        assert_eq!(
            output["daily"][1]["modelBreakdowns"][0]["modelName"],
            json!("claude-opus-4-7")
        );
    }

    #[test]
    fn sort_order_option_reverses_period_rows() {
        let (_, mut rows) = parse_export(CLASSIC_DAILY, path()).unwrap();
        let shared = SharedArgs {
            order: SortOrder::Desc,
            ..SharedArgs::default()
        };
        filter_and_sort_summaries(&mut rows, &shared, |row| {
            row.date.as_deref().unwrap_or_default()
        });

        assert_eq!(rows[0].date.as_deref(), Some("2026-04-16"));
    }

    #[test]
    fn keeps_residual_tokens_beyond_the_counted_fields() {
        // Some ccusage rows report a totalTokens larger than the four counted
        // fields (e.g. reasoning tokens); the residual must survive so the
        // report totals still match the export.
        let export = r#"{"daily": [{"date": "2026-03-30", "inputTokens": 100, "outputTokens": 50, "cacheCreationTokens": 10, "cacheReadTokens": 5, "totalTokens": 200, "totalCost": 0.5, "modelsUsed": [], "modelBreakdowns": []}], "totals": {}}"#;

        let (_, rows) = parse_export(export, path()).unwrap();

        assert_eq!(rows[0].extra_total_tokens, 35);
        assert_eq!(rows[0].total_tokens(), 200);
        assert_eq!(totals_json(&rows)["totalTokens"], json!(200));
    }
}
