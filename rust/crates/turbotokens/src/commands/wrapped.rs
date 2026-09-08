//! `turbotokens wrapped` — a year-in-review summary card of unified agent
//! usage: totals, busiest day, longest streak, top model/project, per-agent
//! split, and favorite day of the week. Terminal card plus a shareable SVG.

use std::collections::BTreeMap;
use std::fs;

use serde_json::{Value, json};
use turbotokens_adapter_all::{DailyAggregate, ProjectAggregate};

use super::visual::{Day, WEEKDAY_NAMES, xml_escape};
use crate::{
    Color, Context as _, Result, cli::WrappedArgs, cli_error, color, format_currency, format_date,
    format_number, format_project_name, print_json_or_jq, utc_now, wants_json,
};

#[derive(Debug)]
struct WrappedStats {
    year: i32,
    total_tokens: u64,
    total_cost: f64,
    active_days: usize,
    busiest_day: Option<DayStat>,
    longest_streak: Option<Streak>,
    top_model: Option<NamedStat>,
    top_project: Option<NamedStat>,
    favorite_weekday: Option<WeekdayStat>,
    agents: Vec<AgentStat>,
}

#[derive(Debug)]
struct DayStat {
    day: Day,
    tokens: u64,
    cost: f64,
}

#[derive(Debug)]
struct Streak {
    days: u64,
    start: Day,
    end: Day,
}

#[derive(Debug)]
struct NamedStat {
    name: String,
    tokens: u64,
    cost: f64,
}

#[derive(Debug)]
struct WeekdayStat {
    name: &'static str,
    tokens: u64,
}

#[derive(Debug)]
struct AgentStat {
    agent: String,
    tokens: u64,
    cost: f64,
    share_percent: f64,
}

pub(super) fn run(args: &WrappedArgs) -> Result<()> {
    let shared = &args.shared;
    if args.year.is_some() && (shared.since.is_some() || shared.until.is_some()) {
        return Err(cli_error("--year cannot be combined with --since/--until"));
    }
    let current_year = format_date(utc_now(), shared.timezone.as_deref())
        .get(..4)
        .and_then(|year| year.parse::<i32>().ok())
        .ok_or_else(|| cli_error("could not resolve the current year"))?;
    let year = args.year.map(|year| year as i32).unwrap_or(current_year);

    let (start, end) = year_window(year, current_year, shared);
    let mut load_shared = shared.clone();
    load_shared.since = Some(start.format_compact());
    load_shared.until = Some(end.format_compact());
    let days = turbotokens_adapter_all::load_daily_aggregates(&load_shared)?;
    let projects = turbotokens_adapter_all::load_project_aggregates(&load_shared)?;
    let stats = compute_stats(year, &days, &projects);

    if let Some(path) = args.svg.as_deref() {
        let svg = render_svg(&stats);
        fs::write(path, &svg).context(format!("Failed to write SVG card to {path}"))?;
        eprintln!("Wrote {path}");
    }

    if wants_json(shared) {
        print_json_or_jq(stats_json(&stats), shared.jq.as_deref(), shared.no_cost)?;
        return Ok(());
    }

    if stats.active_days == 0 {
        eprintln!("No usage data found for {}.", stats.year);
        return Ok(());
    }
    print_card(&stats, shared);
    Ok(())
}

/// The [start, end] days to summarize: the whole year, or the user's
/// --since/--until window (clamped to the year bounds where absent).
fn year_window(year: i32, current_year: i32, shared: &crate::cli::SharedArgs) -> (Day, Day) {
    let year_start = Day::from_ymd(year, 1, 1).unwrap_or(Day::parse("2000-01-01").unwrap());
    let year_end = Day::from_ymd(year, 12, 31).unwrap_or(year_start);
    let today = Day::parse(&format_date(utc_now(), shared.timezone.as_deref())).unwrap_or(year_end);
    let default_end = if year == current_year {
        today
    } else {
        year_end
    };
    let start = shared
        .since
        .as_deref()
        .and_then(|bound| Day::parse(&dashed(bound)))
        .unwrap_or(year_start);
    let end = shared
        .until
        .as_deref()
        .and_then(|bound| Day::parse(&dashed(bound)))
        .unwrap_or(default_end);
    (start, end)
}

/// Turns a normalized compact bound (`YYYYMMDD`) into the dashed form
/// `Day::parse` understands.
fn dashed(bound: &str) -> String {
    if bound.len() == 8 && bound.is_ascii() {
        format!("{}-{}-{}", &bound[..4], &bound[4..6], &bound[6..8])
    } else {
        bound.to_string()
    }
}

fn compute_stats(
    year: i32,
    days: &[DailyAggregate],
    projects: &[ProjectAggregate],
) -> WrappedStats {
    let mut days_sorted: Vec<&DailyAggregate> = days.iter().collect();
    days_sorted.sort_by(|a, b| a.date.cmp(&b.date));

    let total_tokens: u64 = days_sorted.iter().map(|day| day.total_tokens).sum();
    let total_cost: f64 = days_sorted.iter().map(|day| day.total_cost).sum();
    let active_days = days_sorted
        .iter()
        .filter(|day| day.total_tokens > 0)
        .count();

    let busiest_day = days_sorted
        .iter()
        .filter(|day| day.total_tokens > 0)
        .max_by_key(|day| day.total_tokens)
        .and_then(|day| {
            Some(DayStat {
                day: Day::parse(&day.date)?,
                tokens: day.total_tokens,
                cost: day.total_cost,
            })
        });

    let mut models = BTreeMap::<String, (u64, f64)>::new();
    let mut agents = BTreeMap::<String, (u64, f64)>::new();
    let mut weekdays = [0u64; 7];
    for day in &days_sorted {
        for model in &day.models {
            let entry = models.entry(model.model.clone()).or_default();
            entry.0 += model.total_tokens;
            entry.1 += model.total_cost;
        }
        for agent in &day.agents {
            let entry = agents.entry(agent.agent.clone()).or_default();
            entry.0 += agent.total_tokens;
            entry.1 += agent.total_cost;
        }
        if let Some(parsed) = Day::parse(&day.date) {
            weekdays[parsed.weekday()] += day.total_tokens;
        }
    }

    let top_model = models
        .into_iter()
        .filter(|(_, (tokens, _))| *tokens > 0)
        .max_by_key(|(_, (tokens, _))| *tokens)
        .map(|(name, (tokens, cost))| NamedStat { name, tokens, cost });

    let top_project = projects
        .iter()
        .max_by_key(|project| project.total_tokens)
        .map(|project| NamedStat {
            name: format_project_name(&project.project_path, &std::collections::HashMap::new()),
            tokens: project.total_tokens,
            cost: project.total_cost,
        });

    let favorite_weekday = weekdays
        .iter()
        .enumerate()
        .filter(|(_, tokens)| **tokens > 0)
        .max_by_key(|(_, tokens)| *tokens)
        .map(|(index, tokens)| WeekdayStat {
            name: WEEKDAY_NAMES[index],
            tokens: *tokens,
        });

    let mut agent_stats: Vec<AgentStat> = agents
        .into_iter()
        .map(|(agent, (tokens, cost))| AgentStat {
            agent,
            tokens,
            cost,
            share_percent: if total_tokens > 0 {
                tokens as f64 / total_tokens as f64 * 100.0
            } else {
                0.0
            },
        })
        .collect();
    agent_stats.sort_by_key(|agent| std::cmp::Reverse(agent.tokens));

    WrappedStats {
        year,
        total_tokens,
        total_cost,
        active_days,
        busiest_day,
        longest_streak: longest_streak(&days_sorted),
        top_model,
        top_project,
        favorite_weekday,
        agents: agent_stats,
    }
}

/// Longest run of consecutive days with usage. Days sort ascending by date.
fn longest_streak(days: &[&DailyAggregate]) -> Option<Streak> {
    let mut best: Option<(u64, Day, Day)> = None;
    let mut current: Option<(u64, Day, Day)> = None; // (length, start, last)
    for day in days {
        let Some(parsed) = Day::parse(&day.date).filter(|_| day.total_tokens > 0) else {
            current = None;
            continue;
        };
        current = Some(match current {
            Some((length, start, previous)) if parsed.days_since(previous) == 1 => {
                (length + 1, start, parsed)
            }
            _ => (1, parsed, parsed),
        });
        if let Some((length, start, last)) = current
            && best.is_none_or(|(best_length, ..)| length > best_length)
        {
            best = Some((length, start, last));
        }
    }
    best.map(|(days, start, end)| Streak { days, start, end })
}

// --- Terminal card -----------------------------------------------------------

const AGENT_BAR_WIDTH: usize = 20;

fn print_card(stats: &WrappedStats, shared: &crate::cli::SharedArgs) {
    let mut lines: Vec<String> = vec![
        format!(
            "{} tokens · {} · {} active days",
            format_number(stats.total_tokens),
            format_currency(stats.total_cost),
            stats.active_days
        ),
        String::new(),
    ];
    if let Some(day) = &stats.busiest_day {
        lines.push(format!(
            "Busiest day     {} · {} tokens · {}",
            day.day.format(),
            format_number(day.tokens),
            format_currency(day.cost)
        ));
    }
    if let Some(streak) = &stats.longest_streak {
        lines.push(format!(
            "Longest streak  {} days · {} → {}",
            streak.days,
            streak.start.format(),
            streak.end.format()
        ));
    }
    if let Some(model) = &stats.top_model {
        let share = model.tokens as f64 / stats.total_tokens.max(1) as f64 * 100.0;
        lines.push(format!(
            "Top model       {} · {share:.1}% of tokens",
            model.name
        ));
    }
    if let Some(project) = &stats.top_project {
        lines.push(format!(
            "Top project     {} · {} tokens",
            project.name,
            format_number(project.tokens)
        ));
    }
    if let Some(weekday) = &stats.favorite_weekday {
        lines.push(format!(
            "Favorite day    {} · {} tokens",
            weekday.name,
            format_number(weekday.tokens)
        ));
    }
    if !stats.agents.is_empty() {
        lines.push(String::new());
        lines.push("Agents".to_string());
        let name_width = stats
            .agents
            .iter()
            .map(|agent| agent.agent.chars().count())
            .max()
            .unwrap_or(0);
        for agent in &stats.agents {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let filled = ((agent.share_percent / 100.0) * AGENT_BAR_WIDTH as f64).round() as usize;
            lines.push(format!(
                "  {:<name_width$} [{}{}] {:>5.1}% · {} tok · {}",
                agent.agent,
                "█".repeat(filled.min(AGENT_BAR_WIDTH)),
                "░".repeat(AGENT_BAR_WIDTH - filled.min(AGENT_BAR_WIDTH)),
                agent.share_percent,
                format_number(agent.tokens),
                format_currency(agent.cost),
                name_width = name_width
            ));
        }
    }

    let title = format!(" turbotokens wrapped · {} ", stats.year);
    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
        .max(title.chars().count());
    let top = format!(
        "╭{}{}╮",
        title,
        "─".repeat(width + 4 - title.chars().count())
    );
    println!("{}", color(shared, top, Color::Blue));
    for line in &lines {
        let padded = format!("│  {line:<width$}  │", width = width);
        println!("{}", color(shared, padded, Color::Blue));
    }
    println!(
        "{}",
        color(shared, format!("╰{}╯", "─".repeat(width + 4)), Color::Blue)
    );
}

// --- JSON ---------------------------------------------------------------------

fn stats_json(stats: &WrappedStats) -> Value {
    json!({
        "year": stats.year,
        "totalTokens": stats.total_tokens,
        "totalCost": stats.total_cost,
        "activeDays": stats.active_days,
        "busiestDay": stats.busiest_day.as_ref().map(|day| json!({
            "date": day.day.format(),
            "tokens": day.tokens,
            "cost": day.cost,
        })),
        "longestStreak": stats.longest_streak.as_ref().map(|streak| json!({
            "days": streak.days,
            "start": streak.start.format(),
            "end": streak.end.format(),
        })),
        "topModel": stats.top_model.as_ref().map(|model| json!({
            "model": model.name,
            "tokens": model.tokens,
            "cost": model.cost,
        })),
        "topProject": stats.top_project.as_ref().map(|project| json!({
            "project": project.name,
            "tokens": project.tokens,
            "cost": project.cost,
        })),
        "favoriteDayOfWeek": stats.favorite_weekday.as_ref().map(|weekday| json!({
            "day": weekday.name,
            "tokens": weekday.tokens,
        })),
        "agents": stats.agents.iter().map(|agent| json!({
            "agent": agent.agent,
            "tokens": agent.tokens,
            "cost": agent.cost,
            "sharePercent": agent.share_percent,
        })).collect::<Vec<_>>(),
    })
}

// --- SVG card ------------------------------------------------------------------

const SVG_WIDTH: i64 = 1280;
const SVG_HEIGHT: i64 = 640;
const FG: &str = "#111111";
const MUTED: &str = "#666666";
const BLUE: &str = "#111111";
const GREEN: &str = "#111111";

const AGENT_COLORS: [&str; 7] = [
    "#111111", "#3d3d3d", "#5c5c5c", "#7a7a7a", "#999999", "#b8b8b8", "#d6d6d6",
];

fn svg_text(x: i64, y: i64, size: i64, fill: &str, weight: &str, content: &str) -> String {
    format!(
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" fill=\"{fill}\" font-weight=\"{weight}\">{}</text>\n",
        xml_escape(content)
    )
}

/// Compact token count for the card's big number: 1.2B, 45.6M, 123k.
fn compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000_000 {
        format!("{:.1}B", tokens as f64 / 1e9)
    } else if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1e6)
    } else if tokens >= 10_000 {
        format!("{:.0}k", tokens as f64 / 1e3)
    } else {
        format_number(tokens)
    }
}

fn render_svg(stats: &WrappedStats) -> String {
    let extra_legend_rows = stats.agents.len().saturating_sub(1) / 5;
    let height = SVG_HEIGHT + extra_legend_rows as i64 * 28;
    let mut svg = String::with_capacity(8192);
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{SVG_WIDTH}\" height=\"{height}\" viewBox=\"0 0 {SVG_WIDTH} {height}\" font-family=\"Menlo, ui-monospace, monospace\">\n"
    ));
    svg.push_str(&format!(
        "<title>turbotokens wrapped · {}</title>\n",
        stats.year
    ));
    svg.push_str(super::visual::GLASS);

    svg.push_str(&svg_text(48, 96, 28, FG, "600", "turbotokens"));
    svg.push_str(&svg_text(
        48,
        128,
        16,
        MUTED,
        "400",
        &format!("wrapped · {}", stats.year),
    ));

    // Big numbers, left column.
    svg.push_str(&svg_text(
        48,
        250,
        72,
        BLUE,
        "700",
        &compact_tokens(stats.total_tokens),
    ));
    svg.push_str(&svg_text(
        48,
        278,
        16,
        MUTED,
        "400",
        &format!("tokens across {} active days", stats.active_days),
    ));
    svg.push_str(&svg_text(
        48,
        350,
        56,
        GREEN,
        "700",
        &format_currency(stats.total_cost),
    ));
    svg.push_str(&svg_text(48, 378, 16, MUTED, "400", "estimated cost"));

    // Stat list, right column.
    let stat_lines: [(String, String); 5] = [
        (
            "busiest day".to_string(),
            stats
                .busiest_day
                .as_ref()
                .map(|day| format!("{} · {}", day.day.format(), compact_tokens(day.tokens)))
                .unwrap_or_else(|| "—".to_string()),
        ),
        (
            "longest streak".to_string(),
            stats
                .longest_streak
                .as_ref()
                .map(|streak| {
                    format!(
                        "{} days · {} → {}",
                        streak.days,
                        streak.start.format(),
                        streak.end.format()
                    )
                })
                .unwrap_or_else(|| "—".to_string()),
        ),
        (
            "top model".to_string(),
            stats
                .top_model
                .as_ref()
                .map(|model| model.name.clone())
                .unwrap_or_else(|| "—".to_string()),
        ),
        (
            "top project".to_string(),
            stats
                .top_project
                .as_ref()
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "—".to_string()),
        ),
        (
            "favorite day".to_string(),
            stats
                .favorite_weekday
                .as_ref()
                .map(|weekday| weekday.name.to_string())
                .unwrap_or_else(|| "—".to_string()),
        ),
    ];
    for (index, (label, value)) in stat_lines.iter().enumerate() {
        let y = 225 + index as i64 * 52;
        svg.push_str(&svg_text(680, y, 14, MUTED, "400", label));
        let display = crate::truncate_to_width(value, 44);
        svg.push_str(&format!("<g><title>{}</title>\n", xml_escape(value)));
        svg.push_str(&svg_text(680, y + 24, 20, FG, "400", &display));
        svg.push_str("</g>\n");
    }

    // Per-agent split: stacked bar plus legend.
    let bar_y = 500;
    let bar_height = 28;
    let bar_width = SVG_WIDTH - 96;
    if !stats.agents.is_empty() && stats.total_tokens > 0 {
        let mut x = 48;
        for (index, agent) in stats.agents.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let width =
                (agent.tokens as f64 / stats.total_tokens as f64 * bar_width as f64).round() as i64;
            if width <= 0 {
                continue;
            }
            let fill = AGENT_COLORS[index % AGENT_COLORS.len()];
            svg.push_str(&format!(
                "<rect x=\"{x}\" y=\"{bar_y}\" width=\"{width}\" height=\"{bar_height}\" fill=\"{fill}\"/>\n"
            ));
            x += width;
        }
        for (index, agent) in stats.agents.iter().enumerate() {
            let legend_x = 48 + (index % 5) as i64 * 236;
            let legend_y = bar_y + bar_height + 32 + (index / 5) as i64 * 28;
            let fill = AGENT_COLORS[index % AGENT_COLORS.len()];
            svg.push_str(&format!(
                "<rect x=\"{legend_x}\" y=\"{}\" width=\"12\" height=\"12\" fill=\"{fill}\"/>\n",
                legend_y - 11
            ));
            let label = format!("{} {:.0}%", agent.agent, agent.share_percent);
            svg.push_str(&svg_text(legend_x + 18, legend_y, 14, FG, "400", &label));
        }
    }

    svg.push_str(&svg_text(
        48,
        height - 32,
        13,
        MUTED,
        "400",
        "turbotokens · generated from local usage logs",
    ));
    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use turbotokens_adapter_all::{AgentAggregate, ModelAggregate};

    use super::*;

    fn day(
        date: &str,
        tokens: u64,
        cost: f64,
        agents: &[(&str, u64)],
        models: &[(&str, u64)],
    ) -> DailyAggregate {
        DailyAggregate {
            date: date.to_string(),
            total_tokens: tokens,
            total_cost: cost,
            agents: agents
                .iter()
                .map(|(agent, tokens)| AgentAggregate {
                    agent: (*agent).to_string(),
                    total_tokens: *tokens,
                    total_cost: 0.0,
                })
                .collect(),
            models: models
                .iter()
                .map(|(model, tokens)| ModelAggregate {
                    model: (*model).to_string(),
                    total_tokens: *tokens,
                    total_cost: 0.0,
                })
                .collect(),
        }
    }

    fn project(path: &str, tokens: u64) -> ProjectAggregate {
        ProjectAggregate {
            project_path: path.to_string(),
            total_tokens: tokens,
            total_cost: 0.0,
        }
    }

    fn fixture_days() -> Vec<DailyAggregate> {
        vec![
            // A three-day streak (Mon-Wed), a gap, then a two-day streak.
            day(
                "2026-03-02",
                100,
                1.0,
                &[("claude", 80), ("codex", 20)],
                &[("opus", 100)],
            ),
            day(
                "2026-03-03",
                300,
                3.0,
                &[("claude", 300)],
                &[("opus", 200), ("gpt-5", 100)],
            ),
            day("2026-03-04", 200, 2.0, &[("codex", 200)], &[("gpt-5", 200)]),
            day("2026-03-08", 50, 0.5, &[("claude", 50)], &[("opus", 50)]),
            day("2026-03-09", 50, 0.5, &[("claude", 50)], &[("opus", 50)]),
        ]
    }

    #[test]
    fn computes_year_review_stats() {
        let stats = compute_stats(
            2026,
            &fixture_days(),
            &[
                project("/Users/x/ccusage-clone", 900),
                project("/Users/x/other", 100),
            ],
        );

        assert_eq!(stats.total_tokens, 700);
        assert!((stats.total_cost - 7.0).abs() < f64::EPSILON);
        assert_eq!(stats.active_days, 5);
        assert_eq!(
            stats.busiest_day.as_ref().map(|day| day.day.format()),
            Some("2026-03-03".to_string())
        );
        let streak = stats.longest_streak.unwrap();
        assert_eq!(streak.days, 3);
        assert_eq!(streak.start.format(), "2026-03-02");
        assert_eq!(streak.end.format(), "2026-03-04");
        assert_eq!(stats.top_model.unwrap().name, "opus");
        assert_eq!(stats.top_project.unwrap().name, "ccusage-clone");
        // Tue 300 + Wed 200 + Mon(3/2) 100 + Sun 50 + Mon(3/9) 50 → Tuesday.
        assert_eq!(stats.favorite_weekday.unwrap().name, "Tuesday");
        assert_eq!(stats.agents[0].agent, "claude");
        assert!((stats.agents[0].share_percent - 480.0 / 700.0 * 100.0).abs() < 1e-9);
        assert_eq!(stats.agents[1].agent, "codex");
    }

    #[test]
    fn longest_streak_ignores_zero_days_and_unsorted_input() {
        let days = vec![
            day("2026-01-03", 10, 0.0, &[], &[]),
            day("2026-01-01", 10, 0.0, &[], &[]),
            day("2026-01-02", 0, 0.0, &[], &[]),
        ];
        let stats = compute_stats(2026, &days, &[]);
        assert_eq!(stats.longest_streak.unwrap().days, 1);
        assert_eq!(stats.active_days, 2);
    }

    #[test]
    fn empty_input_yields_no_highlights() {
        let stats = compute_stats(2026, &[], &[]);
        assert_eq!(stats.active_days, 0);
        assert!(stats.busiest_day.is_none());
        assert!(stats.longest_streak.is_none());
        assert!(stats.top_model.is_none());
        assert!(stats.favorite_weekday.is_none());
        assert!(stats.agents.is_empty());
    }

    #[test]
    fn json_carries_all_stats() {
        let stats = compute_stats(
            2026,
            &fixture_days(),
            &[project("/Users/x/ccusage-clone", 5)],
        );
        let report = stats_json(&stats);

        assert_eq!(report["year"], json!(2026));
        assert_eq!(report["totalTokens"], json!(700));
        assert_eq!(report["activeDays"], json!(5));
        assert_eq!(report["busiestDay"]["date"], json!("2026-03-03"));
        assert_eq!(report["longestStreak"]["days"], json!(3));
        assert_eq!(report["topModel"]["model"], json!("opus"));
        assert_eq!(report["topProject"]["project"], json!("ccusage-clone"));
        assert_eq!(report["favoriteDayOfWeek"]["day"], json!("Tuesday"));
        assert_eq!(report["agents"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn svg_is_a_well_formed_1280x640_card() {
        let stats = compute_stats(
            2026,
            &fixture_days(),
            &[project("/Users/x/ccusage-clone", 900)],
        );

        let svg = render_svg(&stats);

        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.contains("width=\"1280\" height=\"640\""));
        assert!(svg.contains(crate::commands::visual::GLASS));
        assert!(svg.contains("Menlo"));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains("wrapped · 2026"));
        assert!(svg.contains("ccusage-clone"));
        assert!(svg.contains("claude"));
        assert!(svg.contains("Tuesday"));
    }

    #[test]
    fn svg_escapes_project_names() {
        let stats = compute_stats(2026, &fixture_days(), &[project("/x/<b>&", 1)]);
        let svg = render_svg(&stats);
        assert!(svg.contains("&lt;b&gt;&amp;"));
        assert!(!svg.contains("<b>&"));
    }

    #[test]
    fn compact_token_counts() {
        assert_eq!(compact_tokens(999), "999");
        assert_eq!(compact_tokens(12_345), "12k");
        assert_eq!(compact_tokens(45_600_000), "45.6M");
        assert_eq!(compact_tokens(1_234_567_890), "1.2B");
    }

    #[test]
    fn svg_shows_every_agent_and_expands_for_the_legend() {
        let mut stats = compute_stats(2026, &fixture_days(), &[]);
        stats.agents = (0..18)
            .map(|index| AgentStat {
                agent: format!("agent-{index}"),
                tokens: 10,
                cost: 0.1,
                share_percent: 100.0 / 18.0,
            })
            .collect();
        stats.total_tokens = 180;
        let svg = render_svg(&stats);
        assert!(svg.contains("height=\"724\""));
        assert!(svg.contains("agent-17 6%"));
        assert!(svg.contains("y=\"644\""));
    }

    #[test]
    fn svg_long_unicode_names_fit_and_keep_the_full_title() {
        let name = "プロジェクト".repeat(20);
        assert!(crate::truncate_to_width(&name, 44).chars().count() < 44);
        let stats = compute_stats(2026, &fixture_days(), &[project(&name, 900)]);
        let svg = render_svg(&stats);
        assert!(svg.contains(&format!("<title>{name}</title>")));
        assert!(svg.contains('…'));
    }

    #[test]
    fn svg_replaces_invalid_label_characters_without_changing_json() {
        let name = "name\0\u{1b}\u{fffe}<>&";
        let mut stats = compute_stats(2026, &fixture_days(), &[project(name, 1)]);
        stats.top_model.as_mut().unwrap().name = name.to_string();
        let svg = render_svg(&stats);
        assert!(svg.contains("<title>name\u{fffd}\u{fffd}\u{fffd}&lt;&gt;&amp;</title>"));
        assert!(!svg.contains('\0'));
        assert!(!svg.contains('\u{1b}'));
        assert!(!svg.contains('\u{fffe}'));
        let report = stats_json(&stats);
        assert_eq!(report["topModel"]["model"], json!(name));
        assert_eq!(report["topProject"]["project"], json!(name));
    }
}
