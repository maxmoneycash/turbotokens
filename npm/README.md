<p align="center"><img src="https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/assets/hero.svg" alt="turbotokens: Know what your coding agents spend." width="100%"></p>

**Fast usage reports, a live dashboard, and shareable stats for 18 AI coding agents.** Read the logs already on your machine, including Claude Code, Codex, OpenCode, Gemini, and more. No account or API key needed for local reports.

If turbotokens helps you, [star the repo on GitHub](https://github.com/maxmoneycash/turbotokens) — it helps other developers find it.

## Install

```sh
npm install -g turbotokens
turbotokens
turbotokens live
```

Or run a report with `npx turbotokens`.

This package installs the native Rust binary for your OS and architecture, verifies its SHA-256 checksum, and exposes the `turbotokens` command. Keep npm installation scripts enabled. macOS, Linux, and Windows have both arm64 and x64 release builds.

## See it work

<img src="https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/assets/live-demo.gif" alt="Live dashboard recording using synthetic Claude Code usage events" width="100%">

```sh
turbotokens daily --last 7                  # Recent usage across detected agents
turbotokens claude daily --breakdown        # Claude model costs
turbotokens codex session                   # Codex usage by session
turbotokens daily --json                    # Structured output
turbotokens live --agent grok               # Follow Grok Build while it works
turbotokens live --agent codex              # Follow Codex while it works
turbotokens stream                         # JSON token events, one object per line
turbotokens heatmap --svg heatmap.svg       # Contribution graph
turbotokens wrapped --year 2026 --svg wrapped.svg
turbotokens doctor                         # Data discovery and diagnostics
```

Live mode also supports budget webhooks, JSON events, and Prometheus metrics. See the [full usage guide](https://github.com/maxmoneycash/turbotokens/blob/main/docs/usage.md).

## Measured performance

On an M1 Max, turbotokens 1.1.0 completed a repeated Claude daily report over 3.63 GB of synthetic logs in **12.9 ms**, compared with **3.65 s for ccusage 20.0.20**. Five-run medians, native ARM64 binaries, warmed caches, six files, matching input/output/cache token counts. These are workload-specific results.

<img src="https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/assets/comparison-daily.png" alt="Native daily report comparison against ccusage" width="100%">

[Monthly comparisons with tokscale, raw samples, and reproduction commands](https://github.com/maxmoneycash/turbotokens/blob/main/rust/bench/README.md).

## Usage worth sharing

<img src="https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/assets/wrapped.svg" alt="Yearly summary exported by the CLI; synthetic usage data" width="100%">

Heatmaps and yearly cards are direct SVG exports. Your exports may include project names and usage statistics.

## Data and costs

Reports read local agent logs. Costs are estimates and can differ from your bill. Pricing refreshes, subscription-limit queries, and configured webhooks may use the network; use `--offline` where supported for embedded or cached pricing. These features do not upload your log files.

## Other installs and help

- [Homebrew, shell installer, and standalone downloads](https://github.com/maxmoneycash/turbotokens#get-started)
- [All supported agents and workflow examples](https://github.com/maxmoneycash/turbotokens#eighteen-agents-one-daily-report)
- [Report a bug](https://github.com/maxmoneycash/turbotokens/issues/new?template=bug_report.yml)
- [Contribute](https://github.com/maxmoneycash/turbotokens/blob/main/CONTRIBUTING.md)

MIT. turbotokens began as a fork of [ccusage](https://github.com/ccusage/ccusage) by @ryoppippi; the original attribution is preserved.
