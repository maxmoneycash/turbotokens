//! `turbotokens heatmap` — a GitHub-style contribution heatmap of unified
//! per-day token usage (or cost with `--cost`), in the terminal or as an SVG.

use std::collections::BTreeMap;
use std::fs;

use serde_json::json;
use turbotokens_adapter_all::DailyAggregate;

use super::visual::Day;
use crate::{
    Color, Context as _, Result, cli::HeatmapArgs, cli_error, color, format_currency, format_date,
    format_number, print_json_or_jq, utc_now, wants_json,
};

/// Days of history shown when no `--since` bound is given: one year of weeks.
const DEFAULT_DAYS: i64 = 364;

/// Terminal cell colors, cold to hot; level 0 is a day without usage.
const LEVEL_COLORS: [Color; 5] = [
    Color::Grey,
    Color::Blue,
    Color::Green,
    Color::Yellow,
    Color::Red,
];

/// GitHub-dark cell colors for the SVG, matching the terminal levels.
const LEVEL_FILLS: [&str; 5] = ["#161b22", "#0e4429", "#006d32", "#26a641", "#39d353"];

/// One day in the heatmap window.
#[derive(Debug, Clone, Copy)]
struct DayCell {
    day: Day,
    tokens: u64,
    cost: f64,
}

impl DayCell {
    fn value(&self, by_cost: bool) -> f64 {
        if by_cost {
            self.cost
        } else {
            self.tokens as f64
        }
    }
}

pub(super) fn run(args: &HeatmapArgs) -> Result<()> {
    let shared = &args.shared;
    let today = Day::parse(&format_date(utc_now(), shared.timezone.as_deref()))
        .ok_or_else(|| cli_error("could not resolve today's date"))?;
    let end = shared
        .until
        .as_deref()
        .and_then(parse_bound)
        .unwrap_or(today);
    let start = shared
        .since
        .as_deref()
        .and_then(parse_bound)
        .unwrap_or_else(|| end.checked_add(-DEFAULT_DAYS).unwrap_or(end));
    if start > end {
        return Err(cli_error(format!(
            "--since ({}) is after --until ({})",
            start.format(),
            end.format()
        )));
    }

    let mut load_shared = shared.clone();
    load_shared.since = Some(start.format_compact());
    load_shared.until = Some(end.format_compact());
    let aggregates = turbotokens_adapter_all::load_daily_aggregates(&load_shared)?;
    let cells = dense_cells(start, end, &aggregates);

    if let Some(path) = args.svg.as_deref() {
        write_svg(path, &cells, args.cost)?;
        eprintln!("Wrote {path}");
    }

    if wants_json(shared) {
        let output = json!(
            cells
                .iter()
                .map(|cell| json!({
                    "date": cell.day.format(),
                    "tokens": cell.tokens,
                    "cost": cell.cost,
                }))
                .collect::<Vec<_>>()
        );
        print_json_or_jq(output, shared.jq.as_deref(), shared.no_cost)?;
        return Ok(());
    }

    if aggregates.is_empty() {
        eprintln!(
            "No usage data found between {} and {}.",
            start.format(),
            end.format()
        );
        return Ok(());
    }
    print_terminal_grid(&cells, args.cost, shared);
    Ok(())
}

/// Interprets a normalized (compact) `--since`/`--until` bound as a concrete
/// day. Partial bounds resolve to the first day of the year/month.
fn parse_bound(bound: &str) -> Option<Day> {
    if !bound.is_ascii() {
        return None;
    }
    let dashed = match bound.len() {
        8 => format!("{}-{}-{}", &bound[..4], &bound[4..6], &bound[6..8]),
        6 => format!("{}-{}-01", &bound[..4], &bound[4..6]),
        4 => format!("{bound}-01-01"),
        _ => return None,
    };
    Day::parse(&dashed)
}

/// One cell per day of the window, including days without usage.
fn dense_cells(start: Day, end: Day, aggregates: &[DailyAggregate]) -> Vec<DayCell> {
    let by_date: BTreeMap<Day, (u64, f64)> = aggregates
        .iter()
        .filter_map(|row| {
            Day::parse(&row.date).map(|day| (day, (row.total_tokens, row.total_cost)))
        })
        .collect();
    let mut cells = Vec::new();
    let mut day = start;
    while day <= end {
        let (tokens, cost) = by_date.get(&day).copied().unwrap_or_default();
        cells.push(DayCell { day, tokens, cost });
        let Some(next) = day.checked_add(1) else {
            break;
        };
        day = next;
    }
    cells
}

/// Quartile intensity level 0-4 of a cell against the window's maximum.
/// Token counts are heavy-tailed (the busiest day dwarfs a normal one), so
/// the ratio is square-root scaled to keep the mid levels visible.
fn level(value: f64, max: f64) -> usize {
    if value <= 0.0 || max <= 0.0 {
        return 0;
    }
    let ratio = (value / max).sqrt();
    if ratio <= 0.25 {
        1
    } else if ratio <= 0.5 {
        2
    } else if ratio <= 0.75 {
        3
    } else {
        4
    }
}

/// The Sunday on or before `day` — the first column of the week grid.
fn week_floor(day: Day) -> Day {
    day.checked_add(-(day.weekday() as i64)).unwrap_or(day)
}

/// Month label for each week column, when the month changes vs the previous
/// column. Columns are counted from `grid_start`; a column is labeled by the
/// first in-window day it contains.
fn month_labels(
    window_start: Day,
    grid_start: Day,
    columns: usize,
) -> Vec<Option<(usize, String)>> {
    let mut labels = Vec::with_capacity(columns);
    let mut previous_month = 0;
    for column in 0..columns {
        let label = grid_start
            .checked_add(column as i64 * 7)
            .map(|first| first.max(window_start))
            .and_then(|day| {
                let month = day.month();
                (month != previous_month).then(|| {
                    previous_month = month;
                    (column, day.month_name().to_string())
                })
            });
        labels.push(label);
    }
    labels
}

fn print_terminal_grid(cells: &[DayCell], by_cost: bool, shared: &crate::cli::SharedArgs) {
    let start = cells.first().map(|cell| cell.day).unwrap();
    let end = cells.last().map(|cell| cell.day).unwrap();
    let grid_start = week_floor(start);
    let columns = (end.days_since(grid_start) / 7 + 1) as usize;
    let max = cells
        .iter()
        .map(|cell| cell.value(by_cost))
        .fold(0.0, f64::max);
    let by_day: BTreeMap<Day, &DayCell> = cells.iter().map(|cell| (cell.day, cell)).collect();

    let metric = if by_cost { "Cost" } else { "Token" };
    println!("{metric} heatmap · {} → {}", start.format(), end.format());

    // Month labels, positioned over the column where each month starts.
    let mut label_row = String::from("    ");
    let labels = month_labels(start, grid_start, columns);
    let mut cursor = 0;
    for (column, label) in labels.into_iter().flatten() {
        while cursor < column * 2 {
            label_row.push(' ');
            cursor += 1;
        }
        label_row.push_str(&label);
        cursor += label.len();
    }
    println!("{}", label_row.trim_end());

    for row in 0..7 {
        let day_label = match row {
            1 => "Mon",
            3 => "Wed",
            5 => "Fri",
            _ => "",
        };
        let mut line = format!("{day_label:<4}");
        for column in 0..columns {
            let Some(day) = grid_start.checked_add(column as i64 * 7 + row) else {
                continue;
            };
            match by_day.get(&day) {
                Some(cell) if (start..=end).contains(&day) => {
                    let cell_level = level(cell.value(by_cost), max);
                    line.push_str(&color(shared, "██", LEVEL_COLORS[cell_level]));
                }
                _ => line.push_str("  "),
            }
        }
        println!("{}", line.trim_end());
    }

    let total_tokens: u64 = cells.iter().map(|cell| cell.tokens).sum();
    let total_cost: f64 = cells.iter().map(|cell| cell.cost).sum();
    let legend = (0..5)
        .map(|cell_level| color(shared, "██", LEVEL_COLORS[cell_level]))
        .collect::<Vec<_>>()
        .join("");
    println!();
    println!(
        "Less {legend} More   {} tokens · {}",
        format_number(total_tokens),
        format_currency(total_cost)
    );
}

fn write_svg(path: &str, cells: &[DayCell], by_cost: bool) -> Result<()> {
    let svg = render_svg(cells, by_cost);
    fs::write(path, &svg).context(format!("Failed to write SVG heatmap to {path}"))?;
    Ok(())
}

const SVG_CELL: i64 = 12;
const SVG_GAP: i64 = 3;
const SVG_PITCH: i64 = SVG_CELL + SVG_GAP;
const SVG_LEFT: i64 = 34;
const SVG_TOP: i64 = 30;

fn render_svg(cells: &[DayCell], by_cost: bool) -> String {
    let Some(start) = cells.first().map(|cell| cell.day) else {
        return String::new();
    };
    let end = cells.last().map(|cell| cell.day).unwrap();
    let grid_start = week_floor(start);
    let columns = (end.days_since(grid_start) / 7 + 1) as usize;
    let max = cells
        .iter()
        .map(|cell| cell.value(by_cost))
        .fold(0.0, f64::max);

    let width = SVG_LEFT + columns as i64 * SVG_PITCH + 14;
    let legend_y = SVG_TOP + 7 * SVG_PITCH + 10;
    let height = legend_y + 18;

    let mut svg = String::with_capacity(cells.len() * 90 + 2048);
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" font-family=\"ui-monospace, Menlo, monospace\">\n"
    ));
    svg.push_str(&format!(
        "<rect width=\"{width}\" height=\"{height}\" fill=\"#0d1117\" rx=\"8\"/>\n"
    ));

    // Month labels above the column where each month starts.
    svg.push_str("<g fill=\"#8b949e\" font-size=\"10\">\n");
    for (column, label) in month_labels(start, grid_start, columns)
        .into_iter()
        .flatten()
    {
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"18\">{label}</text>\n",
            SVG_LEFT + column as i64 * SVG_PITCH
        ));
    }
    for (row, label) in [(1, "Mon"), (3, "Wed"), (5, "Fri")] {
        svg.push_str(&format!(
            "<text x=\"4\" y=\"{}\">{label}</text>\n",
            SVG_TOP + row * SVG_PITCH + 10
        ));
    }
    svg.push_str("</g>\n");

    for cell in cells {
        let column = cell.day.days_since(grid_start) / 7;
        let row = cell.day.weekday() as i64;
        let cell_level = level(cell.value(by_cost), max);
        let x = SVG_LEFT + column * SVG_PITCH;
        let y = SVG_TOP + row * SVG_PITCH;
        let fill = LEVEL_FILLS[cell_level];
        let title = if by_cost {
            format!("{} · {}", cell.day.format(), format_currency(cell.cost))
        } else {
            format!(
                "{} · {} tokens",
                cell.day.format(),
                format_number(cell.tokens)
            )
        };
        svg.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{SVG_CELL}\" height=\"{SVG_CELL}\" rx=\"3\" fill=\"{fill}\"><title>{title}</title></rect>\n"
        ));
    }

    // Legend: Less [cells] More, bottom right.
    let legend_width = 30 + 5 * SVG_PITCH + 34;
    let legend_x = width - legend_width - 14;
    svg.push_str("<g font-size=\"10\" fill=\"#8b949e\">\n");
    svg.push_str(&format!(
        "<text x=\"{legend_x}\" y=\"{}\">Less</text>\n",
        legend_y + 10
    ));
    for (cell_level, fill) in LEVEL_FILLS.iter().enumerate() {
        svg.push_str(&format!(
            "<rect x=\"{}\" y=\"{legend_y}\" width=\"{SVG_CELL}\" height=\"{SVG_CELL}\" rx=\"3\" fill=\"{fill}\"/>\n",
            legend_x + 30 + cell_level as i64 * SVG_PITCH
        ));
    }
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\">More</text>\n",
        legend_x + 30 + 5 * SVG_PITCH + 4,
        legend_y + 10
    ));
    svg.push_str("</g>\n</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use turbotokens_adapter_all::DailyAggregate;

    use super::*;

    fn aggregate(date: &str, tokens: u64, cost: f64) -> DailyAggregate {
        DailyAggregate {
            date: date.to_string(),
            total_tokens: tokens,
            total_cost: cost,
            agents: Vec::new(),
            models: Vec::new(),
        }
    }

    #[test]
    fn parses_full_and_partial_bounds() {
        assert_eq!(parse_bound("20260902").unwrap().format(), "2026-09-02");
        assert_eq!(parse_bound("202609").unwrap().format(), "2026-09-01");
        assert_eq!(parse_bound("2026").unwrap().format(), "2026-01-01");
        assert!(parse_bound("20").is_none());
        assert!(parse_bound("notadate").is_none());
    }

    #[test]
    fn dense_cells_fill_gaps_with_zero_days() {
        let cells = dense_cells(
            Day::parse("2026-08-31").unwrap(),
            Day::parse("2026-09-02").unwrap(),
            &[aggregate("2026-09-01", 100, 0.5)],
        );

        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].tokens, 0);
        assert_eq!(cells[1].tokens, 100);
        assert_eq!(cells[2].tokens, 0);
    }

    #[test]
    fn levels_are_sqrt_scaled_quartiles_of_the_max() {
        assert_eq!(level(0.0, 100.0), 0);
        // sqrt scaling: the ratio thresholds are 0.25 / 0.5 / 0.75 after sqrt.
        assert_eq!(level(6.0, 100.0), 1); // sqrt(0.06) ≈ 0.24
        assert_eq!(level(10.0, 100.0), 2); // sqrt(0.10) ≈ 0.32
        assert_eq!(level(25.0, 100.0), 2); // sqrt(0.25) = 0.50
        assert_eq!(level(26.0, 100.0), 3);
        assert_eq!(level(50.0, 100.0), 3);
        assert_eq!(level(75.0, 100.0), 4); // sqrt(0.75) ≈ 0.87
        assert_eq!(level(5.0, 0.0), 0);
    }

    #[test]
    fn month_labels_mark_first_column_of_each_month() {
        let start = Day::parse("2026-08-30").unwrap(); // Sunday
        let cells = dense_cells(start, Day::parse("2026-10-15").unwrap(), &[]);
        let columns = (cells.last().unwrap().day.days_since(start) / 7 + 1) as usize;
        let labels = month_labels(start, start, columns);
        let names: Vec<&str> = labels
            .iter()
            .flatten()
            .map(|(_, name)| name.as_str())
            .collect();

        assert_eq!(names, vec!["Aug", "Sep", "Oct"]);
        // September starts in the column containing Sep 1 (Monday of week 1).
        assert_eq!(labels[0], Some((0, "Aug".to_string())));
        assert_eq!(labels[1], Some((1, "Sep".to_string())));
    }

    #[test]
    fn svg_is_well_formed_and_marks_levels() {
        let cells = dense_cells(
            Day::parse("2026-08-31").unwrap(),
            Day::parse("2026-09-06").unwrap(),
            &[
                aggregate("2026-09-01", 100, 1.0),
                aggregate("2026-09-03", 400, 4.0),
            ],
        );

        let svg = render_svg(&cells, false);

        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.trim_end().ends_with("</svg>"));
        // 7 day cells + background + 5 legend cells.
        assert_eq!(svg.matches("<rect").count(), 13);
        assert!(svg.contains(LEVEL_FILLS[4]), "hottest day uses level 4");
        assert!(svg.contains("<title>2026-09-01 · 100 tokens</title>"));
        assert!(svg.contains("Less"));
        assert!(svg.contains(">Sep</text>"));
        assert!(svg.contains(">Mon</text>"));
    }

    #[test]
    fn svg_titles_use_cost_in_cost_mode() {
        let cells = dense_cells(
            Day::parse("2026-09-01").unwrap(),
            Day::parse("2026-09-01").unwrap(),
            &[aggregate("2026-09-01", 100, 1.5)],
        );

        let svg = render_svg(&cells, true);

        assert!(svg.contains("<title>2026-09-01 · $1.50</title>"));
    }

    #[test]
    fn terminal_and_svg_level_palettes_have_five_levels() {
        // The grid and the legend both rely on exactly five intensity levels.
        assert_eq!(LEVEL_COLORS.len(), 5);
        assert_eq!(LEVEL_FILLS.len(), LEVEL_COLORS.len());
    }
}
