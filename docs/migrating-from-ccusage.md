# Move a ccusage workflow to turbotokens

Start with the command your app runs and compare its output before switching.
turbotokens is a native CLI. Its npm package installs that executable; it does
not expose the JavaScript APIs from older ccusage packages.

## Install alongside ccusage

Follow the [macOS](installation.md#macos), [Windows](installation.md#windows),
or [Linux](installation.md#linux) instructions, then verify the installation:

```text
turbotokens --version
turbotokens claude daily --offline --json
```

Both tools read existing agent logs. You can keep ccusage installed and switch
back without converting those logs. For deployed integrations, pin a tested
release and verify its archive checksum. See the [installation details](installation.md).

## Choose the same source

Unqualified `turbotokens daily` combines detected agents. Use an explicit source
when replacing a Claude-only or Codex-only report.

| Existing workflow | turbotokens command |
| --- | --- |
| Claude daily JSON | `turbotokens claude daily --offline --json` |
| Claude monthly JSON | `turbotokens claude monthly --offline --json` |
| Claude weekly JSON | `turbotokens claude weekly --offline --json` |
| Claude sessions | `turbotokens claude session --offline --json` |
| Claude billing blocks | `turbotokens blocks --offline --json` |
| Codex daily JSON | `turbotokens codex daily --offline --json` |
| Combined agent report | `turbotokens daily --offline --json --by-agent` |
| Claude Code status line | `turbotokens statusline` |

Older ccusage releases used `ccusage daily` for Claude. Newer releases also
have unified reports. Check your pinned version and the fields your app reads.
Source-specific Claude daily rows use `date`; unified rows use `period`.
Claude and Codex also have different cost and model-breakdown fields. A
successful `JSON.parse` alone does not establish compatibility.

Keep date filters, timezone, project, and cost mode explicit. For example:

```sh
ccusage claude daily --since 20260901 --until 20260907 \
  --timezone UTC --offline --json > before.json
turbotokens claude daily --since 20260901 --until 20260907 \
  --timezone UTC --offline --json > after.json
diff -u before.json after.json
```

`--until` includes the named day. Check each command's `--help` before copying
flags. `--last` is relative to the current date, so fixed dates are better for
a reproducible comparison.

## Check the contract your app depends on

Our [executable compatibility check](../rust/bench/verify-ccusage.py) runs
synthetic Claude records through both installed native binaries. It checks
daily, weekly, monthly, session, and block JSON, date and project filters,
duplicates, incomplete appended records, and an empty history. Each case
checks known input, output, cache-write, and cache-read totals. It also compares
cached and uncached turbotokens output byte for byte.

```sh
python3 rust/bench/verify-ccusage.py \
  --turbotokens /path/to/turbotokens \
  --ccusage /path/to/ccusage \
  --output compatibility.json
```

The check uses offline pricing, recorded costs (`--mode display`), explicit
timezones, and temporary data/config/cache directories. It reads no personal
histories. [Recorded results](../rust/bench/results/compatibility-claude.json)
identify the versions and binary hashes tested. This evidence covers those
cases; test additional agents, flags, and schemas used by your application.

Estimated costs can differ between builds with different pricing snapshots.
Compare every token category separately, then verify the cost mode your app
needs. `--mode display` uses recorded costs; `--mode calculate` computes them
from model rates. Neither estimate is a subscription quota or a provider bill.

## Verified app integrations

These 2026-09-08 checks compare the **published [v1.1.3 release](https://github.com/maxmoneycash/turbotokens/releases/tag/v1.1.3)**
with published **ccusage 20.0.20**. Each run verifies the downloaded release
archive and executable against recorded SHA-256 hashes and retains test logs.

| Proposed integration | What is checked | Native platforms | Result |
| --- | --- | --- | --- |
| [token-history](https://github.com/keli-wen/token-history/pull/1) | Claude/Codex snapshots, token and cost fields, dates, timezone, and failure handling | Linux and macOS, x64 and ARM64; Python 3.12 | [27 passed](https://github.com/maxmoneycash/token-history/actions/runs/34267667203) |
| [aimonitor](https://github.com/Loksly/aimonitor/pull/1) | Literal executable paths and matching API/max usage results | Linux x64 and ARM64; Node 24.5.0 | [90 passed](https://github.com/maxmoneycash/aimonitor/actions/runs/34267670974) |
| [ccfleet](https://github.com/tangshunpu/ccfleet/pull/1) | Daily, monthly, session, and block reports through local and cached-mirror execution | Linux x64 and ARM64; Node 20.20.2 and 24.5.0 | [8 passed](https://github.com/maxmoneycash/ccfleet/actions/runs/34267674385) |

The test counts apply to each job in the linked run; all have zero skipped tests. Histories and configuration are
synthetic; pricing is offline. The proposals are under review. The checks cover
the listed collector paths; SSH transport and physical LCD hardware are outside
their scope. See [installation](installation.md) for currently published versions.

## Call the executable from an app

Use an argument array and handle errors before parsing stdout. This Node.js
example runs a bounded Claude report:

```js
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const run = promisify(execFile);
const binary = process.env.TURBOTOKENS_BIN || 'turbotokens';
const { stdout } = await run(binary, [
  'claude', 'daily', '--json', '--offline', '--timezone', 'UTC',
], {
  encoding: 'utf8',
  timeout: 30_000,
  maxBuffer: 16 * 1024 * 1024,
});
const report = JSON.parse(stdout);
if (!Array.isArray(report.daily) ||
    !Number.isFinite(report.totals?.totalTokens)) {
  throw new Error('Unexpected Claude daily report');
}
console.log(report.totals.totalTokens);
```

Use a trusted executable path, supplied by the app's configuration. On Windows,
point it to the native `turbotokens.exe` from a release archive; npm's `.cmd`
shim needs different process-launch handling. Report a failed command, timeout,
or invalid JSON as an error. Substituting zero usage would hide a failed scan.

Install once for scheduled reports. Repeated `npx` package resolution adds work
outside the native CLI timing. A useful integration offers an explicit backend
setting and keeps the existing ccusage command available for rollback.

## Configuration and offline operation

turbotokens discovers `.turbotokens/turbotokens.json` in the working directory
and `turbotokens.json` in Claude config directories. It does not automatically
rename or migrate ccusage config files. Inspect supported options before
copying a config, or pass a reviewed file with `--config /path/to/config.json`.
An empty `{}` file keeps user defaults out of a controlled comparison. From v1.1.3, an explicit
`--config` that cannot be read or is not a JSON object exits with an error before
a report is printed. Surface that error in your app; do not replace it with zero
usage.

`CLAUDE_CONFIG_DIR` chooses Claude history directories. The
[adapter guides](../rust/adapters/README.md) describe other sources.
`TURBOTOKENS_CACHE_DIR` overrides the Claude parse and report cache location.
From v1.1.3, each user gets a platform cache directory unless overridden:

| Platform | Default cache directory |
| --- | --- |
| macOS | `~/Library/Caches/turbotokens` |
| Windows | `%LOCALAPPDATA%\turbotokens` |
| Linux | `$XDG_CACHE_HOME/turbotokens`, or `~/.cache/turbotokens` |

Give independent users separate directories if overriding this location.
In v1.1.3 and later, `TURBOTOKENS_CACHE=off` bypasses both caches, even when a cache directory is set.
The cached daily speed measurements are Claude-specific.

Local reports can use embedded or cached pricing with `--offline` where the
command supports it. Installation downloads, refreshed pricing, subscription
`limits`, and configured webhooks have their own network requirements. JSON
reports can contain session identifiers and project names; use sanitized
fixtures when posting evidence.

If a workflow differs, report the two versions, commands, expected fields, and
a small synthetic or sanitized fixture in a
[bug report](https://github.com/maxmoneycash/turbotokens/issues/new?template=bug_report.yml).
