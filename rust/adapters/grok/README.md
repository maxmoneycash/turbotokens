# Grok Build adapter

The Grok adapter reads completed inference records from Grok Build's global
`logs/unified.jsonl` file and model changes from per-session `events.jsonl`
files. `live.rs` tails those same JSONL files for `turbotokens live --agent grok`.
By default it reads `~/.grok`; set `GROK_HOME` to another Grok data directory.
Comma-separated roots are accepted for audits across copied stores.

```bash
turbotokens grok daily --json
turbotokens grok monthly
turbotokens grok session
turbotokens live --agent grok
turbotokens stream --agent grok
```

Each completed inference record reports the full prompt size, the cached prompt
subset, final completion tokens, and separate reasoning tokens. turbotokens
maps these as fresh input (`prompt - cached`), cache read, output, and an
additional total-token bucket for reasoning. Reasoning is included in cost at
the model's output rate.

Grok's log is global rather than stored inside each session. Reports cover the
inference records still present in `logs/unified.jsonl`; older sessions cannot
be reconstructed when that file does not contain their completed calls.
`turbotokens live --agent grok` and `turbotokens stream --agent grok` tail that
same file. Reasoning tokens are included in live `totalTokens` (folded into
output) so the feed matches Grok report totals.

This adapter reports the application that wrote the records as Grok Build. Its
model breakdown uses the `model_id` recorded for each Grok Build turn, such as
`grok-4.5` or a configured custom model. It does not read Cursor data; Cursor is
a separate application source even when Cursor runs the same model.
