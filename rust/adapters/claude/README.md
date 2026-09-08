# turbotokens-adapter-claude

The Claude Code adapter: it turns the JSONL transcripts Claude Code writes per project and session
into the usage entries the reports render.

## Owns

- `daily.rs` — the daily report path, which reads the same files with a narrower parser.
- `paths.rs` — environment variables, default directories, and file discovery.
- `cache.rs` — on-disk parse cache: per-file scanned entries keyed by path and file identity/change metadata; changed files are rescanned to handle both appends and rewrites.
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

Daily reports use a resident daemon only when its indexed Claude directories
match the current configuration. Older daemons without source identity are
bypassed; restart the daemon after upgrading to use its in-memory index.

Report scans read files into owned buffers so concurrent truncation cannot
invalidate memory while parsing. A scan buffers one file per worker; memory
and uncached read time grow with the largest active files. Repeat reports over
unchanged logs still use the parse and report caches. Concurrent writes can
change the history during a report; rerun after writes settle for stable totals.

Live and resident indexes check file identity/change metadata on each poll,
including Unix inode and change time. When a file changes, they verify its
existing bytes before treating growth as an append. Rewritten,
truncated, deleted, or restored logs rebuild the in-memory totals, including
duplicates that survive in other files. This rebuild does not emit historical
records as new usage; it clears the trailing burn window. Unchanged files need
no content reads, while verifying an append reads the active file's full prefix.
If a surviving file changes during a rebuild read, the previous index stays in
place until a later poll can read every surviving file coherently. Poll intervals
set scan cadence; verification time grows with the active file size.

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
