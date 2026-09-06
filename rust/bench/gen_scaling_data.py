#!/usr/bin/env python3
"""Generate a synthetic Claude-format JSONL dataset totaling N tokens.

Usage: gen_scaling_data.py <out_dir> <billions_of_tokens>
Deterministic (seed 42). Lines are cache-read-heavy like real Claude Code
logs: ~31.5K recorded tokens per ~2.3 KB line. Timestamps spread over 30 days
across 3 projects x 2 sessions.
"""
import json, random, sys, os
from datetime import datetime, timezone

out_dir, billions = sys.argv[1], float(sys.argv[2])
target = int(billions * 1_000_000_000)
rng = random.Random(42)

MODELS = ["claude-opus-5", "claude-fable-5", "claude-opus-4-8"]
PROJECTS = ["webapp", "api", "infra"]
DAY = 86400_000  # ms

# realistic fat transcripts: ~2 KB of message content per line, like real
# Claude Code logs (prompts/tool results make real logs ~1-2 tokens/byte,
# not the 87/byte of bare usage lines)
_SNIPPET_POOL = [
    "".join(rng.choice("abcdefghijklmnopqrstuvwxyz .,;:!?") for _ in range(997)) + "\n"
    for _ in range(64)
]

def content():
    return rng.choice(_SNIPPET_POOL) + rng.choice(_SNIPPET_POOL)

for proj in PROJECTS:
    for sess in ("sess-1", "sess-2"):
        os.makedirs(f"{out_dir}/projects/{proj}", exist_ok=True)

files = [
    open(f"{out_dir}/projects/{proj}/{sess}.jsonl", "w")
    for proj in PROJECTS
    for sess in ("sess-1", "sess-2")
]

total = 0
i = 0
base_ms = 1_780_000_000_000  # fixed epoch, ~2026-06
while total < target:
    f = files[i % len(files)]
    inp = rng.randint(200, 600)
    outp = rng.randint(80, 220)
    cc = rng.randint(500, 1500)
    cr = rng.randint(20_000, 40_000)
    total += inp + outp + cc + cr
    ts = base_ms + (i * 97_000) % (30 * DAY)
    iso = datetime.fromtimestamp(ts / 1000, timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")
    line = {
        "timestamp": iso,
        "version": "2.0.0",
        "sessionId": f"sess-{i % 2 + 1}",
        "message": {
            "id": f"msg-{i}",
            "model": MODELS[i % len(MODELS)],
            "content": content(),
            "usage": {
                "input_tokens": inp,
                "output_tokens": outp,
                "cache_creation_input_tokens": cc,
                "cache_read_input_tokens": cr,
            },
        },
        "requestId": f"req-{i}",
    }
    f.write(json.dumps(line, separators=(",", ":")) + "\n")
    i += 1

for f in files:
    f.close()
with open(f"{out_dir}/TOTAL_TOKENS", "w") as f:
    f.write(str(total))
print(f"{out_dir}: {total:,} tokens, {i:,} lines")
