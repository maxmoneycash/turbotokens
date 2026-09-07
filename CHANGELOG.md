# Changelog

## Unreleased

- Add `turbotokens stream` for a newline-delimited JSON feed of live token events (`live --json` is the same stream). Each usage event includes `totalTokens`.

## 1.1.2 · 2026-09-06

- Restyle heatmap and wrapped SVG exports with a shared Liquid Glass inspired material, brighter edges, and dark text. Report data and terminal output are unchanged.
- Apply the same material to README artwork, comparison charts, terminal previews, and the live recording.

## 1.1.1 · 2026-09-06

- Fix a panic when opening `turbotokens completions --help`. All declared help pages now have a rendering regression test.
- Include the full final calendar day when filtering unified sessions with `--until`, using the requested timezone. This also restores wrapped project statistics for sessions active on the final day. Affected filtered totals can increase because previously omitted sessions are included; report schemas are unchanged.
- Give SVG heatmaps a date range, usage total, and enough space for the legend even in one-day exports.
- Improve contrast and typography in yearly SVG cards. Long project/model labels fit the card and retain their full value in a tooltip; every agent appears in a wrapping legend.
- Publish reproducible native-binary comparisons against ccusage 20.0.20 and tokscale 4.15.1, with raw samples, hashes, and token-category checks.
- Replace README visuals with a consistent theme, direct CLI exports, and a live recording from isolated synthetic data. Expand installation, workflows, integrations, agent coverage, and troubleshooting.
- Generate the release checksum manifest automatically after all six platform builds finish.

See [GitHub releases](https://github.com/maxmoneycash/turbotokens/releases) for earlier versions and downloadable binaries.
