# Usage guide

`turbotokens` defaults to a daily report across detected agents. Add an agent name to read only that source, such as `turbotokens claude daily` or `turbotokens codex session`.

Moving an existing app or script? See [migrating from ccusage](migrating-from-ccusage.md) for command mappings, JSON checks, and a subprocess example.

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

Watch today's tokens and estimated spend in the terminal:

```sh
turbotokens live
turbotokens live --agent codex
turbotokens live --agent grok
turbotokens live --interval 250
```

Live mode currently follows **Claude Code** (default), **Codex**, or **Grok Build**. It polls every 100 ms by default. The time until an event appears also depends on when the agent writes its log. Grok live tails `~/.grok/logs/unified.jsonl` and uses per-session `events.jsonl` files for model names.

Press Ctrl-C to stop.

### Stream token events

For scripts, pipes, and other tools, read a newline-delimited JSON feed:

```sh
turbotokens stream
turbotokens stream --agent grok
turbotokens live --json          # same feed
```

The feed starts with a `type: "snapshot"` object containing today's totals. It
then emits accepted usage records from existing history, followed by records
read as the files grow. Each `type: "usage"` line looks like:

```json
{
  "type": "usage",
  "timestamp": "2026-07-27T18:00:00.000Z",
  "agent": "claude",
  "project": "webapp",
  "sessionId": "sess-1",
  "model": "claude-sonnet-4",
  "inputTokens": 90,
  "outputTokens": 50,
  "cacheCreationTokens": 10,
  "cacheReadTokens": 5,
  "totalTokens": 155,
  "cost": 0.0123
}
```

`agent` is `claude`, `codex`, or `grok`. `totalTokens` is input + output + cache creation + cache read. `cost` is an estimate in USD. A broken pipe (`| head`) is a clean stop.

```sh
# Inspect usage records, excluding the startup snapshot
turbotokens stream | jq --unbuffered -c 'select(.type == "usage") | {time: .timestamp, model, tokens: .totalTokens, cost}'
```

Filter on `type` before consuming records. Adding the startup snapshot to the
usage records would count historical usage twice. Claude replacement records
can also correct an earlier message without emitting another usage event, so
summing this feed does not reconstruct current totals. Use a fresh JSON report
or the [Prometheus gauges](#prometheus-metrics) for aggregate usage.

On a TTY, `turbotokens live` is the dashboard. Piped or `--json` / `stream` is the machine feed.

### Budget alerts

```sh
turbotokens live --alert-cost 25 --webhook https://example.com/usage-alerts
turbotokens stream --alert-tokens 1000000 --webhook https://example.com/usage-alerts
```

The webhook receives a JSON body. Alerts also print on stderr when streaming so stdout stays a clean event feed:

```json
{
  "type": "alert",
  "metric": "cost",
  "threshold": 25,
  "value": 25.4,
  "date": "2026-09-07"
}
```

`metric` is `cost` or `tokens`. Services such as Slack or Discord need a small adapter. Alerts notify you; they do not stop the coding agent.

### Prometheus metrics

```sh
turbotokens live --serve 127.0.0.1:9090
turbotokens stream --serve 127.0.0.1:9090
curl -s http://127.0.0.1:9090/metrics
```

Gauges for today's usage:

| Metric | Meaning |
| --- | --- |
| `turbotokens_tokens_total{kind=...}` | Today's tokens by `input`, `output`, `cache_creation`, `cache_read` |
| `turbotokens_cost_usd_total` | Today's estimated cost in USD |
| `turbotokens_tokens_per_minute` | Burn rate over the trailing 5 minutes |
| `turbotokens_model_tokens_total{model=...}` | Today's tokens by model |
| `turbotokens_sessions_active` | Sessions with activity in the last 5 minutes |
| `turbotokens_files_watched` | JSONL log files currently tracked |

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

Use `TURBOTOKENS_CACHE_DIR` to choose a separate cache directory. On Unix, a resident index daemon can serve compatible Claude daily reports from memory:

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

## Claude Code status line

Merge this entry into Claude Code's `settings.json`, preserving your other settings:

```json
{
  "statusLine": {
    "type": "command",
    "command": "turbotokens statusline"
  }
}
```

Claude Code sends the command a JSON description of the active session on standard input. The output includes usage and cost information. Offline pricing is the default; `--visual-burn-rate text` adds a burn-rate indicator. Run `turbotokens statusline --help` for cost-source, context-threshold, and cache options.
