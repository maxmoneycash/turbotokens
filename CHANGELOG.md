# Changelog

## 1.1.3 · 2026-09-08

- Add `turbotokens stream` for a newline-delimited JSON feed of live token events (`live --json` is the same stream). Each usage event includes `totalTokens`.
- Add `turbotokens live --agent grok` / `stream --agent grok`, tailing Grok Build's `logs/unified.jsonl`.
- Include `agent` on live/stream usage events and snapshots (`claude`, `codex`, or `grok`).
- Restyle README graphics and native SVG exports: drop the glass frame and tracked type, use a flat page, and put terminal captures on a dark field.

- Keep Claude cached reports and live/daemon totals correct after rewrites, truncation, deletion/restoration, and duplicate winners moving between dates. Cache validation includes Unix and Windows change metadata.
- Bound derived report caches to 256 slots per report kind. Full-key validation turns slot collisions into cache misses; repeated updates no longer leave a file for every old history.
- Read mutable logs into owned buffers to avoid a crash when a file is truncated during an uncached scan. Uncached scans do more copying; unchanged reports still use both cache layers.
- Accept whitespace-formatted Claude and Codex JSON records without changing report schemas. Reject unreadable or malformed explicit configuration before printing a report.
- Isolate daemon IPC under the user's home directory and verify source, timezone, and cost context before reuse. Stop daemons through their verified protocol.
- Escape log-supplied terminal controls and give metrics responses an absolute write deadline.
- Fix npm installation from Git Bash on Windows by selecting the native ZIP extractor even when another `tar` appears first on PATH.
- Verify shell-install checksums and staged executables before replacing an install. Forward npm launcher shutdown signals and wait for the native child.
- Ship static Linux binaries checked on Ubuntu 22.04, Debian 12, and Alpine 3.22, for x64 and ARM64. All six native release targets now check report counts, cache invalidation, live rewrites, and deterministic log mutations.
- Add clearer macOS, Windows, and Linux installation instructions, a ccusage migration guide, and standalone documentation inside release archives.

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
