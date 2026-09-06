# Usage guide

`turbotokens` defaults to a daily report across detected agents. Add an agent name to read only that source, such as `turbotokens claude daily` or `turbotokens codex session`.

## Reports

| Command | Grouping or purpose |
| --- | --- |
| `daily` | Calendar day |
| `weekly` | Calendar week |
| `monthly` | Calendar month |
| `session` | Agent session |
| `blocks` | Claude usage in five-hour billing blocks |
| `limits` | Available Claude and Codex subscription windows and reset times |
| `doctor` | Detected data, cache health, and pricing diagnostics |

```sh
turbotokens daily --since 20260901 --until 20260930
turbotokens daily --timezone America/Los_Angeles
turbotokens daily --json --by-agent
turbotokens claude daily --breakdown
turbotokens daily --sections daily,weekly,monthly --json
```

`--since` and `--until` accept dates; `--until` is inclusive. Unified reports also accept hyphenated dates. Flags vary by command and agent. Use the relevant `--help` to check availability.

The token total can include input, output, cache creation, and cache reads, depending on the source. Costs use recorded costs or model rates according to the adapter and calculation mode. Missing usage or unknown model pricing can make a report incomplete. Inspect `doctor` and the source agent's output when totals look unexpected.

## Live mode

```sh
turbotokens live
turbotokens live --agent codex
turbotokens live --interval 250
```

Live mode defaults to Claude Code and a 100 ms polling interval. Actual display latency depends on when the agent writes its log, the amount of data, and the machine.

Stream newline-delimited JSON:

```sh
turbotokens live --json
```

Send a budget alert to an endpoint that accepts JSON:

```sh
turbotokens live --alert-cost 25 --webhook https://example.com/usage-alerts
```

The webhook receives turbotokens' alert payload. Services with a different payload format need an adapter.

Expose Prometheus metrics locally:

```sh
turbotokens live --serve 127.0.0.1:9090
```

Press Ctrl-C to stop live mode.

## Heatmap and yearly summary

```sh
turbotokens heatmap
turbotokens heatmap --cost --since 20260101 --until 20261231
turbotokens heatmap --svg heatmap.svg
turbotokens wrapped --year 2026
turbotokens wrapped --year 2026 --svg wrapped.svg
```

The heatmap defaults to the past year. Wrapped reports token totals, estimated cost, active days, the busiest day, longest streak, top model and project when available, and usage by agent. Both support `--json`.

SVG exports contain usage statistics. Review project names and other visible details before sharing them.

## Import an existing report

Render a ccusage daily, monthly, or session JSON export:

```sh
turbotokens import report.json
```

Import uses the exported report rather than reading the original agent logs.

## Repeated reports

The Claude adapter caches parsing results automatically. To bypass this cache for diagnosis:

```sh
TURBOTOKENS_CACHE=off turbotokens claude daily --offline --json
```

Use `TURBOTOKENS_CACHE_DIR` to choose a separate cache directory. For repeated unified reports, a resident index daemon is also available:

```sh
turbotokens daemon start
turbotokens daemon status
turbotokens daemon stop
```

See `turbotokens daemon --help` for its lifecycle commands.

## Shell completion

```sh
turbotokens completions zsh
```

The command prints a completion script. Install it in the completion directory used by your shell.
