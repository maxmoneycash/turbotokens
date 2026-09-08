<p align="center">
  <picture>
    <source media="(max-width: 600px)" srcset="assets/hero-mobile.svg">
    <img src="assets/hero.svg" alt="turbotokens. Know what your agents spend." width="100%">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/maxmoneycash/turbotokens/releases/latest"><img src="https://img.shields.io/github/v/release/maxmoneycash/turbotokens?style=flat&color=222&labelColor=555" alt="Latest release"></a>
  <a href="https://www.npmjs.com/package/turbotokens"><img src="https://img.shields.io/npm/v/turbotokens?style=flat&color=222&labelColor=555&label=npm" alt="npm version"></a>
  <a href="https://github.com/maxmoneycash/turbotokens/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/maxmoneycash/turbotokens/ci.yml?branch=main&style=flat&color=222&labelColor=555" alt="CI status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-222?style=flat&labelColor=555" alt="MIT license"></a>
</p>

**Fast usage reports for Claude Code, Codex, and 16 more coding agents.** See token counts and estimated cost, follow sessions live, or export JSON for your app. A native Rust CLI that reads the logs already on your machine. Local reports need no account or API key.

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#coming-from-ccusage">Migrate from ccusage</a> ·
  <a href="#reports">Reports</a> ·
  <a href="#live-dashboard">Live dashboard</a> ·
  <a href="#benchmarks">Benchmarks</a> ·
  <a href="docs/usage.md">Docs</a>
</p>

<a id="get-started"></a>

## Install

Choose your platform. Native downloads are available for **Apple Silicon / ARM64 and Intel / AMD x64**.

### macOS

With [Homebrew](https://brew.sh) installed:

```sh
brew install maxmoneycash/tap/turbotokens
```

For a standalone install, follow the [macOS instructions](docs/installation.md#macos). No Node.js runtime is needed.

### Windows

With Node.js installed, run this in **PowerShell** or **Command Prompt**:

```powershell
npm install -g turbotokens
turbotokens --version
```

For a native install without Node.js, download the [Windows x64 ZIP](https://github.com/maxmoneycash/turbotokens/releases/latest/download/turbotokens-windows-x64.zip) or [Windows ARM64 ZIP](https://github.com/maxmoneycash/turbotokens/releases/latest/download/turbotokens-windows-arm64.zip). Extract it, open PowerShell in that folder, and run `./turbotokens.exe doctor`. The [Windows guide](docs/installation.md#windows) explains checksums, PATH, and WSL.

### Linux

Run in your terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/install.sh | sh
```

The installer selects x64 or ARM64 and prints the install location. If it uses `~/.local/bin`, add that directory to your PATH:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

The current v1.1.2 Linux release requires glibc 2.39. See [Linux instructions](docs/installation.md#linux) for distro compatibility, persistent PATH setup, and pinned releases. No Node.js runtime is needed.

### Run your first report

These commands work on all three platforms after installation:

```sh
turbotokens --version
turbotokens doctor                  # Find your agents' logs
turbotokens                         # Daily usage across detected agents
turbotokens claude daily --last 7    # Claude usage over the last seven days
turbotokens codex daily              # Focus on Codex
```

You can also try **`npx turbotokens`** on any platform with Node.js. The npm package downloads and launches the native executable; keep npm installation scripts enabled. [All install and upgrade options →](docs/installation.md)

## Coming from ccusage?

Install alongside it, choose the same agent, and compare the fields your app reads:

```sh
# Claude-only JSON, suitable for a Claude usage integration
turbotokens claude daily --offline --json

# Codex-only JSON
turbotokens codex daily --offline --json
```

**Use an explicit agent when replacing a single-source report.** Plain `turbotokens daily` combines detected agents and has a different JSON shape. The npm package exposes a CLI; it does not replace older ccusage JavaScript imports.

The [migration guide](docs/migrating-from-ccusage.md) includes command mappings, a Node.js subprocess example, rollback advice, and an executable compatibility check. [Seventeen recorded cases](rust/bench/results/compatibility-claude.json) compare released native binaries on synthetic Claude histories, including token categories, filters, duplicates, and cached versus uncached JSON.

<a id="eighteen-agents-one-daily-report"></a>

## Reports

<img src="assets/daily-report.png?v=e50811eee970" alt="A turbotokens daily report with separate input, output, cache, total, and estimated cost columns. Synthetic Claude Code and Codex data." width="100%">

| Find out | Run |
| :--- | :--- |
| Usage by model | `turbotokens claude daily --breakdown` |
| Which Claude project accounts for the spend | `turbotokens claude daily --instances` |
| Usage by session | `turbotokens codex session` |
| Monthly trends | `turbotokens monthly --since 20260101` |
| Current Claude billing block | `turbotokens blocks --active` |
| Each agent's share, as JSON | `turbotokens daily --json --by-agent` |

Filter by date, project, or timezone where the source supports it. Run `turbotokens <agent> <command> --help` for that report's options.

<details>
<summary><strong>All 18 supported agents</strong></summary>

| | | |
| :--- | :--- | :--- |
| [Claude Code](rust/adapters/claude) · `claude` | [Codex](rust/adapters/codex) · `codex` | [OpenCode](rust/adapters/opencode) · `opencode` |
| [Amp](rust/adapters/amp) · `amp` | [Gemini CLI](rust/adapters/gemini) · `gemini` | [GitHub Copilot](rust/adapters/copilot) · `copilot` |
| [Kimi](rust/adapters/kimi) · `kimi` | [Grok Build](rust/adapters/grok) · `grok` | [Qwen](rust/adapters/qwen) · `qwen` |
| [Droid](rust/adapters/droid) · `droid` | [Codebuff](rust/adapters/codebuff) · `codebuff` | [Hermes](rust/adapters/hermes) · `hermes` |
| [Goose](rust/adapters/goose) · `goose` | [Kilo](rust/adapters/kilo) · `kilo` | [OpenClaw](rust/adapters/openclaw) · `openclaw` |
| [pi-agent](rust/adapters/pi) · `pi` | [Antigravity](rust/adapters/antigravity) · `antigravity` | [ZCode](rust/adapters/zcode) · `zcode` |

Agent formats differ. Available model, project, cache, and session details depend on what each agent records. Live mode currently supports Claude Code, Codex, and Grok Build. [Adapter notes →](rust/adapters/README.md)

</details>

<a id="watch-your-agents-work"></a>

## Live dashboard

```sh
turbotokens live                    # Claude Code
turbotokens live --agent codex
turbotokens live --agent grok
```

<img src="assets/live-demo.gif?v=28ea241e0f51" alt="The turbotokens terminal dashboard updating token counts, estimated cost, burn rate, and active sessions as synthetic usage records arrive." width="100%">

Live mode polls every 100 ms by default. It shows new usage as the agent writes its logs. [Static preview](assets/live-dashboard.png) · [Recording source](demo/README.md)

For scripts and monitoring:

```sh
turbotokens stream --offline                 # One JSON usage event per line
turbotokens live --alert-cost 25              # Notify at $25 estimated spend
turbotokens live --serve 127.0.0.1:9090        # Local Prometheus endpoint
```

Budget alerts can call a webhook; they do not stop the coding agent. See [live options and payloads](docs/usage.md#live-mode), or add `turbotokens statusline` to your [Claude Code status line](docs/usage.md#claude-code-status-line).

<a id="your-usage-worth-sharing"></a>

## Your year in AI coding

```sh
turbotokens wrapped --year 2026 --svg wrapped.svg
```

<img src="assets/wrapped.svg?v=8282dcf9c767" alt="A CLI-exported yearly usage card with token totals, estimated cost, active days, longest streak, top model, and agent shares. Synthetic sample data." width="100%">

Export your yearly summary as an SVG for a README, a personal site, or the [show-and-tell thread](https://github.com/maxmoneycash/turbotokens/discussions/6). Check project names before sharing your own card.

<details>
<summary><strong>Prefer a contribution heatmap?</strong></summary>

```sh
turbotokens heatmap --svg heatmap.svg
```

<img src="assets/heatmap.svg?v=92961b162223" alt="A CLI-exported daily usage heatmap with token totals and contribution cells. Synthetic sample data." width="100%">

Add `--cost` to color by estimated spend, or omit `--svg` for a terminal view. Both heatmap and wrapped support JSON export.

</details>

All product images use synthetic data. The SVG previews are direct CLI exports.

<a id="see-the-speed"></a>

## Benchmarks

**A cached Claude daily report took 12.9 ms on 3.63 GB of logs, versus 3.65 s for ccusage.** This is a measured repeat-report workload, with no daemon running.

<img src="assets/comparison-daily.png?v=d161a6f2d5d1" alt="Cached daily JSON report benchmark across 72 MB to 3.63 GB of synthetic Claude logs. Full measurements and reproduction commands are linked below." width="100%">

Measured September 6, 2026, on an Apple M1 Max with native ARM64 releases: turbotokens **1.1.0** and ccusage **20.0.20**. Five interleaved runs per size; median wall time includes JSON output. Pricing and caches were primed, and all four token categories matched on each run. This measures recorded usage counters, not text tokenization.

The cache is Claude-specific. First scans, changed files, other agents, and different machines take different paths. With 1,000 smaller files (72 MB total), the measured daily medians were 12.7 ms and 71.9 ms.

**[Commands, raw samples, versions, and binary hashes →](rust/bench/README.md)**

<details>
<summary><strong>Monthly reports and the first scan</strong></summary>

### Monthly reports: compared with both tools

<img src="assets/comparison-monthly.png?v=fe6de784eae5" alt="Monthly JSON report benchmark on 3.63 GB of synthetic Claude logs: turbotokens 1.54 seconds, tokscale 3.99 seconds, and ccusage 4.83 seconds. Lower is better." width="100%">

| Transcript size | turbotokens 1.1.0 | ccusage 20.0.20 | tokscale 4.15.1 |
| :--- | ---: | ---: | ---: |
| 72 MB | **38 ms** | 71 ms | 107 ms |
| 362 MB | **147 ms** | 273 ms | 340 ms |
| 725 MB | **263 ms** | 539 ms | 751 ms |
| 1.81 GB | **622 ms** | 1,230 ms | 1,859 ms |
| 3.63 GB | **1.54 s** | 4.83 s | 3.99 s |

**Measured September 6, 2026, on an Apple M1 Max.** Five runs per tool and size, interleaved; median process wall time includes JSON output. Each dataset contains six synthetic files. The largest contains usage counters totaling 50 billion tokens; it is **3.63 GB of transcript data**, not 50 billion tokens of text being tokenized. Monthly reporting uses a different aggregation path from the cached daily fast path.

Different file counts, agent formats, machines, and background load can change the result. Costs can differ with pricing sources, so the cross-tool check compares token categories, not estimated dollars. Tokscale has no equivalent `daily` report command; it is included in the matching monthly comparison.

**[Raw samples, binary hashes, and reproduction commands →](rust/bench/README.md)**

<details>
<summary>What about the first scan?</summary>

In a separate cache-on/cache-off test, turbotokens scanned the 3.63 GB history in **1.10 s with its parse cache disabled**, then returned the cached daily report in **9 ms**. Both used a warmed OS file cache and offline pricing. Every cached and uncached JSON result was byte-identical.

<img src="assets/scaling-chart.png?v=14d98ff6804c" alt="turbotokens daily report timing with its parse cache enabled and disabled across five transcript sizes" width="100%">

See the [cache benchmark](rust/bench/README.md#cache-enabled-versus-disabled) for the separate run and its raw samples.

</details>

</details>

## Data, costs, and privacy

- **Your logs stay local.** Reports aggregate usage records already on your machine. They need no transcript upload or turbotokens account.
- **Costs are estimates.** Model prices and recorded usage can differ from a provider bill or subscription allowance. Missing local logs cannot be reconstructed.
- **Network access depends on the feature.** Use `--offline` where supported for embedded or cached pricing. Installation, refreshed pricing, subscription `limits`, and configured webhooks contact their respective services.

An empty or surprising report starts with `turbotokens doctor`. Check the source, date range, timezone, and [adapter's log locations](rust/adapters/README.md). For a counting issue, share a small sanitized fixture in a [bug report](https://github.com/maxmoneycash/turbotokens/issues/new?template=bug_report.yml).

## Go further

| Guide | What it covers |
| :--- | :--- |
| [Install and upgrade](docs/installation.md) | macOS, Windows, Linux, checksums, PATH, and pinned releases |
| [Migrate from ccusage](docs/migrating-from-ccusage.md) | Commands, JSON contracts, subprocesses, and reproducible checks |
| [Usage guide](docs/usage.md) | Filters, live events, alerts, status line, completion, and the optional daemon |
| [Adapter notes](rust/adapters/README.md) | Supported records and source-specific behavior |
| [Contributing](CONTRIBUTING.md) | Build, test, and report a reproducible bug |

<details>
<summary><strong>Build from source</strong></summary>

```sh
git clone https://github.com/maxmoneycash/turbotokens.git
cd turbotokens/rust
cargo build --release --bin turbotokens --features fetch-litellm-pricing
cargo test --workspace --features fetch-litellm-pricing
cargo clippy --release --bin turbotokens --features fetch-litellm-pricing
```

The repository pins its Rust toolchain. Builds embed a pricing snapshot; set `TURBOTOKENS_PRICING_JSON_PATH` to use a local LiteLLM pricing file. Parsing and cache changes must preserve report JSON, with fixtures and measured performance checks.

</details>

If turbotokens is useful to you, a star helps other developers find it. Contributions with real workloads and reproducible counting bugs are welcome.

## License and credits

[MIT](LICENSE). turbotokens began as a fork of [ccusage](https://github.com/ccusage/ccusage) by [@ryoppippi](https://github.com/ryoppippi). The original copyright is preserved alongside the rewrite's attribution. Thanks to the ccusage contributors and the projects whose local usage formats make these reports possible.
