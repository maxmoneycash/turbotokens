<p align="center">
  <picture>
    <source media="(max-width: 600px)" srcset="assets/hero-mobile.svg?v=7c1b74544e24">
    <img src="assets/hero.svg?v=bbd500d105d5" alt="turbotokens: Know what your coding agents spend. Native Rust CLI for 18 AI coding agents." width="100%">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/maxmoneycash/turbotokens/releases/latest"><img src="https://img.shields.io/github/v/release/maxmoneycash/turbotokens?style=flat&color=222&labelColor=555" alt="Latest release"></a>
  <a href="https://github.com/maxmoneycash/turbotokens/stargazers"><img src="https://img.shields.io/github/stars/maxmoneycash/turbotokens?style=flat&color=222&labelColor=555" alt="GitHub stars"></a>
  <a href="https://www.npmjs.com/package/turbotokens"><img src="https://img.shields.io/npm/v/turbotokens?style=flat&color=222&labelColor=555&label=npm" alt="npm version"></a>
  <a href="https://www.npmjs.com/package/turbotokens"><img src="https://img.shields.io/npm/dm/turbotokens?style=flat&color=222&labelColor=555&label=downloads" alt="npm downloads per month"></a>
  <a href="https://github.com/maxmoneycash/turbotokens/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/maxmoneycash/turbotokens/ci.yml?branch=main&style=flat&color=222&labelColor=555" alt="CI status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-222?style=flat&labelColor=555" alt="MIT license"></a>
</p>

**Fast usage reports, a live dashboard, and shareable stats for your AI coding agents.** turbotokens reads the logs already on your machine. Check your spend across Claude Code, Codex, and 16 more agents with one command. No account or API key needed for local reports.

If turbotokens saved you from a surprise bill, a ⭐ helps other developers find it.

<p align="center">
  <a href="#get-started">Get started</a> ·
  <a href="#see-the-speed">Benchmarks</a> ·
  <a href="#watch-your-agents-work">Live dashboard</a> ·
  <a href="#your-usage-worth-sharing">Usage cards</a> ·
  <a href="docs/migrating-from-ccusage.md">Migrate from ccusage</a> ·
  <a href="docs/usage.md">Usage guide</a>
</p>

## Get started

```sh
brew install maxmoneycash/tap/turbotokens

turbotokens         # Daily usage across your detected agents
turbotokens live    # Follow Claude Code as it works
turbotokens stream  # Same data as newline-delimited JSON
```

Already have Node.js? Run a report with **`npx turbotokens`**, or install the command with `npm install -g turbotokens`.

<details>
<summary><strong>More ways to install: shell, Windows, and standalone binaries</strong></summary>

On macOS or Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/install.sh | sh
```

[Download a native binary](https://github.com/maxmoneycash/turbotokens/releases/latest) for **macOS, Linux, or Windows**, on **arm64 or x64**. Extract the archive and put `turbotokens` on your PATH. Release assets include a SHA-256 checksum manifest.

The Homebrew and standalone installs need no Node.js runtime. The npm package is a small launcher that downloads the matching native binary; keep npm installation scripts enabled. The badges above show the currently available GitHub and npm versions.

After installing:

```sh
turbotokens --version
turbotokens doctor
```

`doctor` shows which agent histories were found and checks pricing and cache health. See [installation and release details](packaging/README.md).

</details>

<img src="assets/live-demo.gif?v=28ea241e0f51" alt="The real turbotokens live dashboard updating as synthetic usage events arrive" width="100%">

## Why turbotokens?

- **Check the whole picture.** Daily, weekly, monthly, and session reports across 18 agents. Drill down by agent, model, project, or date where the source supports it.
- **Keep reports quick as history grows.** Claude daily reports cache parsed usage. Our 3.63 GB benchmark took **13 ms on a repeat run**, versus **3.65 s for ccusage** with the same logs.
- **See a session while it is happening.** A live Claude Code, Codex, or Grok Build dashboard shows usage, estimated cost, burn rate, active sessions, and new events.
- **Put the numbers to work.** `turbotokens stream` for JSON events, budget webhooks, Prometheus metrics, JSON reports, and a Claude Code status line.
- **Keep a record you can share.** Export a contribution heatmap or a yearly summary as an SVG. The CLI generates both.

The timings are medians on a specific synthetic workload, not a promise for every machine or agent. The full comparison is below.

## See the speed

We benchmark **released native ARM64 binaries**, including the current Rust versions of ccusage and tokscale. Every tool reads the same Claude-format logs. Caches and pricing are primed before timing, and input, output, cache creation, and cache read totals must agree on every run.

### Daily reports: 13 ms versus 3.65 seconds

<img src="assets/comparison-daily.png?v=d161a6f2d5d1" alt="Daily JSON report benchmark: turbotokens stays between 11 and 17 milliseconds across 72 MB to 3.63 GB of synthetic logs; ccusage takes 78 milliseconds to 4.49 seconds. Full measurements are linked below." width="100%">

At **3.63 GB**, turbotokens completed a cached daily report in **12.9 ms**, compared with **3,649 ms** for ccusage: about **283× faster in this test**. Both commands use `claude daily --offline --json`. No daemon is running.

A separate check across **1,000 smaller files** (72 MB total) took **12.7 ms** for turbotokens and **71.9 ms** for ccusage. File count and workload shape matter; the [raw results](rust/bench/results/comparison-many-files.json) include this case too.

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

## Eighteen agents. One daily report.

Run `turbotokens` to combine the sources it finds, or name an agent to focus on it.

<img src="assets/daily-report.png?v=e50811eee970" alt="A real turbotokens daily report generated from synthetic Claude Code and Codex sessions, with separate input, output, cache, total, and cost columns" width="100%">

```sh
turbotokens claude daily --breakdown    # Claude usage, including model costs
turbotokens codex session               # Codex usage by session
turbotokens daily --json --by-agent     # Keep each agent separate in JSON
turbotokens monthly --since 20260101    # This year's monthly totals
```

| | | |
| :--- | :--- | :--- |
| [Claude Code](rust/adapters/claude) · `claude` | [Codex](rust/adapters/codex) · `codex` | [OpenCode](rust/adapters/opencode) · `opencode` |
| [Amp](rust/adapters/amp) · `amp` | [Gemini CLI](rust/adapters/gemini) · `gemini` | [GitHub Copilot](rust/adapters/copilot) · `copilot` |
| [Kimi](rust/adapters/kimi) · `kimi` | [Grok Build](rust/adapters/grok) · `grok` | [Qwen](rust/adapters/qwen) · `qwen` |
| [Droid](rust/adapters/droid) · `droid` | [Codebuff](rust/adapters/codebuff) · `codebuff` | [Hermes](rust/adapters/hermes) · `hermes` |
| [Goose](rust/adapters/goose) · `goose` | [Kilo](rust/adapters/kilo) · `kilo` | [OpenClaw](rust/adapters/openclaw) · `openclaw` |
| [pi-agent](rust/adapters/pi) · `pi` | [Antigravity](rust/adapters/antigravity) · `antigravity` | [ZCode](rust/adapters/zcode) · `zcode` |

Agent formats differ. Available model, project, cache, and session details depend on what each agent records. Live mode currently supports Claude Code, Codex, and Grok Build. [Adapter notes →](rust/adapters/README.md)

## Watch your agents work

```sh
turbotokens live
turbotokens live --agent codex
turbotokens live --agent grok
turbotokens stream                 # JSON event feed
```

<img src="assets/live-dashboard.png?v=ede55d90c946" alt="The live dashboard showing estimated cost, tokens, burn rate, active sessions, and recent usage events; synthetic data" width="100%">

Watch today's tokens and estimated spend, the current burn rate, active sessions, and recent events update in the terminal. Live mode polls every **100 ms** by default; `--interval 250` changes that to 250 ms. The time until an event appears also depends on when the agent writes its logs.

[Static dashboard preview](assets/live-dashboard.png) · [Recording source and reproduction](demo/README.md)

### Get a budget alert

```sh
turbotokens live \
  --alert-cost 25 \
  --webhook https://your-endpoint.example/usage-alerts
```

Send an alert when today's estimated cost crosses your threshold. Use `--alert-tokens` for a token budget. Webhooks receive a JSON payload; services such as Slack or Discord need a small adapter for their message format. Alerts notify you; they do not stop the coding agent.

### Feed a dashboard or script

```sh
# Newline-delimited JSON events (one usage object per line)
turbotokens stream
turbotokens stream --agent grok | jq -c '{agent, model, tokens: .totalTokens, cost}'

# Prometheus metrics on your machine
turbotokens live --serve 127.0.0.1:9090
curl -s http://127.0.0.1:9090/metrics

# Save a report for another tool
turbotokens daily --json > usage.json
```

[Live options and integration details →](docs/usage.md#live-mode)

## Your usage, worth sharing

### A contribution graph for your coding agents

```sh
turbotokens heatmap --svg heatmap.svg
```

<img src="assets/heatmap.svg?v=92961b162223" alt="CLI-generated daily usage heatmap with date range, token total, and green contribution cells; synthetic sample data" width="100%">

See your usage streaks and busy weeks. Add `--cost` to color by estimated spend, set a date range with `--since` and `--until`, or leave off `--svg` to see the heatmap in your terminal.

### Your year in AI coding

```sh
turbotokens wrapped --year 2026 --svg wrapped.svg
```

<img src="assets/wrapped.svg?v=8282dcf9c767" alt="CLI-generated yearly summary showing token totals, estimated cost, active days, longest streak, top model and project, and Claude and Codex shares; synthetic sample data" width="100%">

Total tokens, active days, estimated cost, busiest day, longest streak, favorite weekday, top model, top project, and your agent split. SVGs stay crisp when resized and work in a README or personal site. Both commands also export JSON.

**Share yours.** Post your card in the [show-and-tell thread](https://github.com/maxmoneycash/turbotokens/discussions/6), or [share it on X](https://twitter.com/intent/tweet?text=My%20year%20in%20AI%20coding%2C%20visualized%20with%20turbotokens%3A%20https%3A%2F%2Fgithub.com%2Fmaxmoneycash%2Fturbotokens). Usage cards may include project names, so check the SVG before posting.

All product images here use **synthetic data**. Heatmap and wrapped previews are direct CLI exports. Your own exports contain your usage statistics and may include project names.

## Make it part of your workflow

| What you want to know | Command |
| :--- | :--- |
| What did I use over the last seven days? | `turbotokens daily --last 7` |
| How does usage change week to week? | `turbotokens weekly` |
| Which Claude project accounts for the spend? | `turbotokens claude daily --instances` |
| What happened in one project? | `turbotokens claude daily --project webapp` |
| How much of the current Claude block have I used? | `turbotokens blocks --active` |
| When do my subscription windows reset? | `turbotokens limits` |
| Can I group days in my own timezone? | `turbotokens daily --timezone America/Los_Angeles` |
| Can I combine report periods in one export? | `turbotokens daily --sections daily,weekly,monthly --json` |
| Can I read an existing ccusage export? | `turbotokens import report.json` |
| Where are my logs, and why is data missing? | `turbotokens doctor` |

Flags vary by agent. `turbotokens <agent> <command> --help` lists the options for that report.

### Put usage in your Claude Code status line

Merge this into your Claude Code `settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "turbotokens statusline"
  }
}
```

The status-line command reads Claude Code's JSON input and uses offline pricing by default. Add `--visual-burn-rate text` to show a text burn-rate indicator. [Status-line options](docs/usage.md#claude-code-status-line)

### Speed up frequent Claude reports

The parse cache works automatically. On Unix, an optional resident daemon can keep the Claude daily index in memory:

```sh
turbotokens daemon start
turbotokens claude daily
turbotokens daemon status
turbotokens daemon stop
```

Compatible Claude daily reports use the daemon; other requests use the normal loader. The benchmarks above do **not** use it. [Cache and daemon details](docs/usage.md#repeated-reports)

### Add shell completion

```sh
turbotokens completions zsh
```

Print completions for your shell and install the result in its completion directory. Bash, Zsh, and Fish are supported.

## How it compares

These are different approaches to the same problem. Here's where each fits, based on the September 6, 2026 releases and documentation.

| | turbotokens | [ccusage](https://github.com/ccusage/ccusage) | [tokscale](https://github.com/junhoyeo/tokscale) |
| :--- | :--- | :--- | :--- |
| Main experience | Fast reports and live usage telemetry | Detailed usage reports and billing blocks | Interactive analytics TUI and sharing platform |
| Agent coverage | 18 | 18 | 50+ client filters |
| Live view | Claude Code, Codex, and Grok Build event dashboard | Claude billing-block monitor | Interactive multi-client TUI |
| Local reports without an account | Yes | Yes | Yes |
| JSON output | Reports and live events | Reports | Reports and graph export |
| Shareable visuals | SVG heatmap and yearly card | Tables and exports | Contribution graphs and wrapped images |
| Built-in integrations | Status line, budget webhooks, Prometheus | Claude Code status line | Social submission and provider quota views |
| Implementation tested | Native Rust | Native Rust | Native Rust |

Choose turbotokens if quick repeated reports and scriptable live monitoring fit your workflow. Tokscale offers broader client coverage and a richer interactive exploration experience. ccusage has extensive reporting documentation and is the project turbotokens grew from.

[Comparison scope and pinned sources →](rust/bench/README.md#comparison-scope)

## Frequently asked questions

### What is a faster alternative to ccusage?

turbotokens began as a fork of ccusage and adds a parse cache for repeated Claude reports. On a 3.63 GB benchmark history, a cached daily report took 13 ms versus 3.65 s for ccusage on the same logs. It also adds a live dashboard, budget alerts, Prometheus metrics, and SVG usage cards. The [migration guide](docs/migrating-from-ccusage.md) covers command mappings, JSON differences, and reproducible compatibility checks. Details: [benchmarks](#see-the-speed).

### How do I track Claude Code costs in real time?

Run `turbotokens live`. It follows Claude Code's local logs as they change and shows the day's usage, estimated cost, burn rate, and active sessions. `turbotokens stream` emits the same events as newline-delimited JSON for scripts, and `turbotokens statusline` puts the numbers in your Claude Code status line. Codex and Grok Build are supported too.

### Can I check my AI coding usage without an account or API key?

Yes. turbotokens reads the usage logs your agents already write to your machine. Use `--offline` where supported to use embedded or cached pricing without a network lookup. Local reports need no account, API key, or transcript upload. Costs are estimates; see [data, costs, and privacy](#data-costs-and-privacy).

### Which agents does turbotokens support?

Eighteen, including Claude Code, Codex, Grok Build, Gemini CLI, GitHub Copilot, and Kimi. One command reports across every agent it detects on your machine; `turbotokens doctor` shows what was found. See the [usage guide](docs/usage.md) for the full list and per-agent flags.

## Data, costs, and privacy

**Local reports read local files.** You do not need to create a turbotokens account or upload transcripts. The CLI aggregates the usage records your agents have already written; it does not estimate tokens by re-tokenizing your conversations.

**Costs are estimates.** Recorded usage and model pricing may not match your bill, especially with subscriptions, provider discounts, unknown models, or incomplete logs. Cache reads and writes are shown separately when the source records them. `limits` queries Claude and Codex subscription information using your existing sign-in credentials; those allowances are separate from estimated API cost.

**Network access is feature-specific.** Pricing lookups may contact external services. Use `--offline` where supported for embedded or cached rates. `limits` contacts the provider, and a webhook sends alert data to your chosen endpoint. These features do not upload your log files.

<details>
<summary><strong>Troubleshooting and common questions</strong></summary>

**My report is empty.** Run `turbotokens doctor`, then the focused agent command. The agent must have written supported usage records on this machine. Custom data directories may need an environment variable; see that [adapter's notes](rust/adapters).

**My totals differ from the provider dashboard.** Check the date range, timezone, model, and token categories. Subscription allowances and estimated API costs are different measurements. Missing local logs cannot be reconstructed from a report.

**Does the speed claim apply to every agent?** No. The benchmarks measure synthetic Claude logs. The daily parse cache and resident daemon described here are Claude-specific.

**Can I use it offline?** Local reports support offline pricing where the command exposes `--offline`. Installation, refreshed pricing, subscription limits, and configured webhooks need their respective network services.

**Can I migrate from ccusage?** Start with the [migration guide](docs/migrating-from-ccusage.md): choose the same agent, preserve filters and timezone, and compare the fields your app reads. It includes a subprocess example and an executable compatibility check. You can also render an existing JSON export with `turbotokens import report.json`.

**What if I find a counting bug?** [Open a bug report](https://github.com/maxmoneycash/turbotokens/issues/new?template=bug_report.yml) with the command, version, `doctor` output, and a small sanitized fixture. Reproducible counting bugs are especially valuable.

</details>

## Build it. Improve it.

```sh
git clone https://github.com/maxmoneycash/turbotokens.git
cd turbotokens/rust
cargo build --release --bin turbotokens --features fetch-litellm-pricing
cargo test --workspace --features fetch-litellm-pricing
cargo clippy --release --bin turbotokens --features fetch-litellm-pricing
```

Use a Rust toolchain supporting edition 2024. The build embeds a pricing snapshot; set `TURBOTOKENS_PRICING_JSON_PATH` to use a local LiteLLM pricing file.

Good contributions include missing log formats, counting fixtures, clearer diagnostics, better terminal output, and reproducible performance improvements. Parsing and cache changes must preserve JSON output; hot paths stay small and avoid an async runtime.

[Contributing guide](CONTRIBUTING.md) · [Adapter architecture](rust/adapters/README.md) · [Benchmarks](rust/bench/README.md) · [Report a bug](https://github.com/maxmoneycash/turbotokens/issues/new?template=bug_report.yml) · [Request a feature](https://github.com/maxmoneycash/turbotokens/issues/new?template=feature_request.yml)

If turbotokens helps you, star the repo, share a usage card, or tell us which agent you want supported next. Real workloads and good bug reports make the tool better.

## Star history

<a href="https://star-history.com/#maxmoneycash/turbotokens&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=maxmoneycash/turbotokens&type=Date&theme=dark">
    <img src="https://api.star-history.com/svg?repos=maxmoneycash/turbotokens&type=Date" alt="Chart of turbotokens GitHub stars over time" width="100%">
  </picture>
</a>

## License and credits

[MIT](LICENSE). turbotokens began as a fork of [ccusage](https://github.com/ccusage/ccusage) by [@ryoppippi](https://github.com/ryoppippi). The original copyright is preserved alongside the rewrite's attribution. Thanks to the ccusage contributors and the projects whose local usage formats make these reports possible.
