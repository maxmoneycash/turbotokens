# turbotokens-adapter-claude

The Claude Code adapter: it turns the JSONL transcripts Claude Code writes per project and session
into the usage entries the reports render.

## Owns

- `daily.rs` — the daily report path, which reads the same files with a narrower parser.
- `paths.rs` — environment variables, default directories, and file discovery.
- `cache.rs` — on-disk parse cache: per-file scanned entries keyed by path/size/mtime; changed files are rescanned to handle both appends and rewrites.
- `live.rs` — the `turbotokens live` real-time telemetry stream (dashboard, NDJSON, and human-line output modes).

Anything that is not specific to this source belongs in `turbotokens-core` or
`turbotokens-adapter-common` instead.

## Data source

- `~/.claude/projects/**/*.jsonl` and `~/.config/claude/projects/**/*.jsonl`

Record shapes, token mapping, and cost rules are documented in [`src/README.md`](src/README.md).

Reads plain files through `turbotokens-adapter-common`, which handles walking, size-balanced
chunking, and ordered parallel reads.

## Parse cache

Unchanged files reuse cached records; changed files are rescanned. The default
cache lives in `~/Library/Caches/turbotokens` on macOS,
`$XDG_CACHE_HOME/turbotokens` (or `~/.cache/turbotokens`) on other Unix systems,
and `%LOCALAPPDATA%/turbotokens` on Windows. Without a usable home directory,
automatic caching is disabled.

`TURBOTOKENS_CACHE_DIR` selects a trusted cache directory explicitly.
`TURBOTOKENS_CACHE=off` disables the cache. New cache directories and files use
Unix permissions `0700` and `0600`, respectively. `turbotokens doctor` reports
the selected location. Existing temporary caches are left untouched.

## Public surface

- `paths::timestamp_from_line`
- `paths::claude_paths`
- `paths::extract_project`
- `paths::extract_session_parts`
- `paths::usage_files`
- `load_entries`
- `load_daily_summaries`
- `run_live`
- `usage_limit_reset_time_from_line`

## Depends on

- `turbotokens-adapter-common`
- `turbotokens-core`
- `jiff`
- `memchr`
- `rustc-hash`
- `serde`
- `serde_json`

## Build layer

Built in the `adapters` Crane artifact layer; the layer compiles all adapters in one Cargo invocation, so they build concurrently.
