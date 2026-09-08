#!/usr/bin/env python3
"""Check released Claude CLI contracts against ccusage on synthetic data.

Uses recorded costs (--mode display), offline pricing, explicit timezones, and
isolated data/config/cache paths. Requires Python 3.9+ and two installed native
binaries. Reads no personal histories. This is a compatibility check, not a
performance benchmark or a guarantee for every ccusage version and command.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import tempfile


KEYS = ("inputTokens", "outputTokens", "cacheCreationTokens", "cacheReadTokens")
ALL_COUNTS = (1000, 140, 100, 50)


def record(index, timestamp, counts, project="migration-a"):
    return {
        "type": "assistant", "timestamp": timestamp, "version": "2.0.0",
        "sessionId": project, "requestId": "request-%s" % index,
        "costUSD": index / 100,
        "message": {
            "id": "message-%s" % index, "model": "claude-sonnet-4-20250514",
            "usage": dict(zip(("input_tokens", "output_tokens",
                               "cache_creation_input_tokens",
                               "cache_read_input_tokens"), counts)),
        },
    }


def line(value):
    return json.dumps(value, separators=(",", ":")) + "\n"


def totals(report):
    if "totals" in report:
        result = report["totals"]
        counts = tuple(result[key] for key in KEYS)
        assert result["totalTokens"] == sum(counts), "totalTokens omits a category"
        return counts, result["totalCost"]
    blocks = report["blocks"]
    block_keys = ("inputTokens", "outputTokens", "cacheCreationInputTokens",
                  "cacheReadInputTokens")
    for block in blocks:
        assert block["totalTokens"] == sum(block["tokenCounts"].values())
    return (tuple(sum(row["tokenCounts"][key] for row in blocks)
                  for key in block_keys), sum(row["costUSD"] for row in blocks))


def run(binary, arguments, environment, directory):
    result = subprocess.run([str(binary), *arguments], env=environment,
                            cwd=directory, capture_output=True, timeout=30)
    if result.returncode:
        raise RuntimeError("%s %s exited %s: %s" % (
            binary.name, " ".join(arguments), result.returncode,
            result.stderr.decode("utf-8", errors="replace")[:1000]))
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--turbotokens", type=Path, required=True)
    parser.add_argument("--ccusage", type=Path, required=True)
    parser.add_argument("--output", type=Path,
                        help="Write versioned, synthetic-only JSON evidence")
    args = parser.parse_args()
    binaries = {name: getattr(args, name).resolve()
                for name in ("turbotokens", "ccusage")}
    metadata = {}
    checks = []
    with tempfile.TemporaryDirectory(prefix="turbotokens-contract-") as temporary:
        root = Path(temporary)
        data = root / "claude"
        project = data / "projects" / "migration-a"
        project.mkdir(parents=True)
        other = data / "projects" / "migration-b"
        other.mkdir()
        config = root / "config.json"
        config.write_text("{}\n")
        environment = dict(os.environ, CLAUDE_CONFIG_DIR=str(data),
                           TURBOTOKENS_CACHE_DIR=str(root / "cache"),
                           TURBOTOKENS_CACHE="on", TZ="UTC", NO_COLOR="1")
        first = record(1, "2025-01-06T00:30:00.000Z", (100, 20, 10, 5))
        second = record(2, "2025-01-06T23:30:00.000Z", (200, 30, 20, 10))
        third = record(3, "2025-01-07T12:00:00.000Z", (300, 40, 30, 15))
        fourth = record(4, "2025-01-07T13:00:00.000Z", (400, 50, 40, 20),
                        "migration-b")
        source = project / "migration-a.jsonl"
        source.write_text(line(first) + line(second) + line(third) + line(first)
                          + "not json\n" + line({"type": "user"}))
        (other / "migration-b.jsonl").write_text(line(fourth))

        for name, binary in binaries.items():
            metadata[name] = {
                "version": run(binary, ["--version"], environment, root).decode().strip(),
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            }

        def check(name, command, expected=ALL_COUNTS, cost=0.1, zone="UTC"):
            arguments = [*command, "--json", "--offline", "--mode", "display",
                         "--timezone", zone, "--config", str(config)]
            raw = {tool: run(binary, arguments, environment, root)
                   for tool, binary in binaries.items()}
            parsed = {tool: json.loads(value) for tool, value in raw.items()}
            for tool, report in parsed.items():
                actual, actual_cost = totals(report)
                assert actual == expected, "%s/%s counts: %r != %r" % (
                    name, tool, actual, expected)
                assert math.isclose(actual_cost, cost, abs_tol=1e-12), (
                    "%s/%s recorded cost: %s != %s" % (name, tool, actual_cost, cost))
            assert parsed["turbotokens"] == parsed["ccusage"], name + ": JSON differs"
            warm = run(binaries["turbotokens"], arguments, environment, root)
            uncached = run(binaries["turbotokens"], arguments,
                           dict(environment, TURBOTOKENS_CACHE="off"), root)
            assert raw["turbotokens"] == warm == uncached, name + ": cache changed JSON bytes"
            checks.append({
                "name": name, "arguments": arguments[:-2],
                "expected_tokens": dict(zip(KEYS, expected)),
                "expected_total_tokens": sum(expected), "expected_recorded_cost": cost,
                "json_equal": True, "json_bytes_equal": raw["turbotokens"] == raw["ccusage"],
                "cache_bytes_equal": True,
                "report": parsed["turbotokens"],
            })
            print("PASS " + name, flush=True)

        for period in ("daily", "weekly", "monthly", "session"):
            check(period, ["claude", period])
        check("blocks", ["blocks"])
        check("daily descending", ["claude", "daily", "--order", "desc"])
        check("daily breakdown", ["claude", "daily", "--breakdown"])
        check("daily instances", ["claude", "daily", "--instances"])
        check("daily since", ["claude", "daily", "--since", "20250107"],
              (700, 90, 70, 35), 0.07)
        check("daily until inclusive", ["claude", "daily", "--until", "20250106"],
              (300, 50, 30, 15), 0.03)
        check("daily exact date", ["claude", "daily", "--since", "20250106",
                                   "--until", "20250106"], (300, 50, 30, 15), 0.03)
        check("daily project", ["claude", "daily", "--project", "migration-a"],
              (600, 90, 60, 30), 0.06)
        check("daily missing project", ["claude", "daily", "--project", "missing"],
              (0, 0, 0, 0), 0)
        check("daily Los Angeles", ["claude", "daily"], zone="America/Los_Angeles")
        dates = [row["date"] for row in checks[-1]["report"]["daily"]]
        assert dates == ["2025-01-05", "2025-01-06", "2025-01-07"], dates

        # A growing JSONL file must not count an unfinished record. Complete it
        # after a cached report to exercise incremental invalidation too.
        fifth = line(record(5, "2025-01-08T12:00:00.000Z", (500, 60, 50, 25)))
        cut = len(fifth) // 2
        with source.open("a") as stream:
            stream.write(fifth[:cut])
        check("partial appended line", ["claude", "daily"])
        with source.open("a") as stream:
            stream.write(fifth[cut:])
        check("completed appended line", ["claude", "daily"], (1500, 200, 150, 75), 0.15)

        empty = root / "empty-claude"
        (empty / "projects").mkdir(parents=True)
        environment["CLAUDE_CONFIG_DIR"] = str(empty)
        check("empty history", ["claude", "daily"], (0, 0, 0, 0), 0)

    evidence = {
        "measured_at": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(), "tools": metadata,
        "scope": "Synthetic Claude logs; explicit source, timezone, offline pricing, recorded cost. Full JSON equality and cache byte parity. No timing claims.",
        "checks": checks,
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(evidence, indent=2) + "\n")
    print("%s compatibility checks passed." % len(checks))


if __name__ == "__main__":
    main()
