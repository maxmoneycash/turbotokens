#!/usr/bin/env python3
"""Stress an installed Claude daily reporter against a deterministic count oracle.

Python 3.9+, standard library only. Every mutation checks cached, warm, and
uncached JSON. All histories, configuration, caches, and subprocesses belong
to this run. This checks report correctness, not throughput or live telemetry.
"""

import argparse
from collections import Counter, deque
from dataclasses import dataclass, replace
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import traceback


KEYS = ("inputTokens", "outputTokens", "cacheCreationTokens", "cacheReadTokens")
RAW_KEYS = ("input_tokens", "output_tokens", "cache_creation_input_tokens",
            "cache_read_input_tokens")
OPERATIONS = ("append", "append_whitespace", "partial_completion",
              "rewrite_same_size", "rewrite_larger", "truncate",
              "delete_restore", "duplicate_across_files")
MAX_RECORDS_PER_FILE = 64
CHILD_TIMEOUT = 15


@dataclass(frozen=True)
class Record:
    index: int
    timestamp: str
    model: str
    counts: tuple
    cost_ticks: int

    def line(self, whitespace=False):
        value = {
            "type": "assistant", "timestamp": self.timestamp,
            "sessionId": "synthetic-stress", "requestId": "req-%012d" % self.index,
            "costUSD": self.cost_ticks / 64,
            "message": {
                "id": "msg-%012d" % self.index, "role": "assistant",
                "model": self.model, "usage": dict(zip(RAW_KEYS, self.counts)),
            },
        }
        separators = (", ", ": ") if whitespace else (",", ":")
        body = json.dumps(value, separators=separators)
        return ((" \t" + body + " \t" if whitespace else body) + "\n").encode()


@dataclass
class Source:
    path: Path
    entries: list
    exists: bool = True

    def content(self):
        return b"".join(record.line(style) for record, style in self.entries)

    def write(self):
        # In-place writes deliberately retain the inode for rewrite cases.
        with self.path.open("wb") as stream:
            stream.write(self.content())
        self.exists = True


def empty_counts():
    return dict.fromkeys(KEYS, 0)


def oracle(sources):
    """Count current logical records, never cumulative generation history.

    Replayed copies share an immutable record ID. Rewrites create new IDs.
    This ledger is independent of JSON parsing and the reporter's cache.
    """
    seen = set()
    days = {}
    total = empty_counts()
    total_ticks = 0
    for source in sources:
        if not source.exists:
            continue
        for record, _style in source.entries:
            if record.index in seen:
                continue
            seen.add(record.index)
            date = record.timestamp[:10]
            day = days.setdefault(date, {"counts": empty_counts(), "cost_ticks": 0})
            for key, value in zip(KEYS, record.counts):
                day["counts"][key] += value
                total[key] += value
            day["cost_ticks"] += record.cost_ticks
            total_ticks += record.cost_ticks

    def row(counts, ticks):
        return dict(counts, totalTokens=sum(counts.values()), totalCost=ticks / 64)

    return {
        "daily": {date: row(day["counts"], day["cost_ticks"])
                  for date, day in sorted(days.items())},
        "totals": row(total, total_ticks), "unique_records": len(seen),
    }


def validate_report(raw, expected, label):
    report = json.loads(raw)
    if not isinstance(report, dict) or not isinstance(report.get("daily"), list):
        raise AssertionError(label + ": expected daily array")
    rows = {}
    for entry in report["daily"]:
        date = entry["date"]
        if date in rows:
            raise AssertionError(label + ": duplicate daily bucket " + date)
        rows[date] = entry
    if set(rows) != set(expected["daily"]):
        raise AssertionError(label + ": daily buckets differ from oracle")
    pairs = [("totals", report["totals"], expected["totals"])]
    pairs.extend((date, rows[date], value) for date, value in expected["daily"].items())
    for bucket, actual, wanted in pairs:
        for key in (*KEYS, "totalTokens"):
            if actual[key] != wanted[key]:
                raise AssertionError("%s/%s/%s: %r != %r" % (
                    label, bucket, key, actual[key], wanted[key]))
        cost = actual["totalCost"]
        if not math.isfinite(cost) or not math.isclose(
                cost, wanted["totalCost"], rel_tol=0, abs_tol=1e-12):
            raise AssertionError("%s/%s/totalCost: %r != %r" % (
                label, bucket, cost, wanted["totalCost"]))


class StressRun:
    def __init__(self, args, root):
        self.args = args
        self.root = root
        self.rng = random.Random(args.seed)
        self.index = 0
        self.step = 0
        self.steps_completed = 0
        self.checkpoints = 0
        self.processes = 0
        self.operations = Counter()
        self.recent = deque(maxlen=32)
        self.last_results = {}
        self.last_expected = {}
        self.report_cache = None
        self.peak_report_files = 0
        self.before = {}
        self.started = time.monotonic()
        self.last_progress = self.started
        self.binary = root / ("turbotokens.exe" if os.name == "nt" else "turbotokens")
        shutil.copy2(args.turbotokens, self.binary)
        self.binary_hash = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.harness = root / "stress-claude.py"
        shutil.copy2(Path(__file__).resolve(), self.harness)
        self.harness_hash = hashlib.sha256(self.harness.read_bytes()).hexdigest()
        self.home = root / "home"
        claude = self.home / ".claude"
        projects = claude / "projects"
        self.sources = []
        for index in range(6):
            path = projects / ("project-%d" % (index % 3)) / ("session-%d.jsonl" % index)
            path.parent.mkdir(parents=True, exist_ok=True)
            source = Source(path, [])
            source.write()
            self.sources.append(source)
        for name in ("config", "cache", "tmp", "codex"):
            (root / name).mkdir()
        self.config = root / "config" / "neutral.json"
        self.config.write_text("{}\n")
        self.environment = {
            "PATH": os.defpath, "HOME": str(self.home), "USERPROFILE": str(self.home),
            "CLAUDE_CONFIG_DIR": str(claude), "CODEX_HOME": str(root / "codex"),
            "XDG_CONFIG_HOME": str(root / "config"), "XDG_CACHE_HOME": str(root / "cache"),
            "TURBOTOKENS_CACHE_DIR": str(root / "cache"), "TURBOTOKENS_CACHE": "on",
            "TZ": "UTC", "NO_COLOR": "1", "TMPDIR": str(root / "tmp"),
            "TEMP": str(root / "tmp"), "TMP": str(root / "tmp"),
        }
        for key in ("SYSTEMROOT", "WINDIR", "COMSPEC"):
            if key in os.environ:
                self.environment[key] = os.environ[key]
        self.arguments = ["claude", "daily", "--json", "--offline", "--mode", "display",
                          "--timezone", "UTC", "--config", str(self.config)]
        self.version = "unknown"

    def run_command(self, arguments, cache):
        self.processes += 1
        started = time.monotonic()
        command = [str(self.binary), *arguments]
        with subprocess.Popen(command, cwd=self.home, env=dict(
                self.environment, TURBOTOKENS_CACHE=cache),
                stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
            timed_out = False
            try:
                stdout, stderr = child.communicate(timeout=CHILD_TIMEOUT)
            except subprocess.TimeoutExpired:
                timed_out = True
                child.kill()
                stdout, stderr = child.communicate()
            except BaseException:
                child.kill()
                child.communicate()
                raise
        return {"stdout": stdout, "stderr": stderr, "returncode": child.returncode,
                "timed_out": timed_out, "seconds": time.monotonic() - started}

    def new_record(self, previous=None):
        self.index += 1
        if previous is not None:
            counts = list(previous.counts)
            # Change a final digit without changing the JSON byte length.
            counts[0] = (counts[0] // 10) * 10 + (counts[0] + 1) % 10
            return replace(previous, index=self.index, counts=tuple(counts))
        return Record(
            self.index,
            "2025-01-%02dT%02d:%02d:00.000Z" % (
                self.rng.randint(6, 9), self.rng.randrange(24), self.rng.randrange(60)),
            self.rng.choice(("claude-sonnet-4-20250514", "claude-opus-4-20250514")),
            tuple(self.rng.randrange(10000) for _ in KEYS), self.rng.randrange(65),
        )

    def check(self, phase, source=None, **details):
        entry = {"step": self.step, "phase": phase, **details}
        if source is not None:
            entry["file"] = str(source.path.relative_to(self.root))
        self.recent.append(entry)
        self.last_expected = oracle(self.sources)
        self.last_results = {}
        # Capture all three before asserting, so a failure distinguishes a
        # scanner error from stale cache data and saves the uncached evidence.
        for label, mode in (("cached", "on"), ("warm", "on"), ("uncached", "off")):
            self.last_results[label] = self.run_command(self.arguments, mode)
        for label, result in self.last_results.items():
            if result["timed_out"] or result["returncode"]:
                raise AssertionError("%s: child exit %s (timeout=%s)" % (
                    label, result["returncode"], result["timed_out"]))
            validate_report(result["stdout"], self.last_expected, label)
        outputs = [result["stdout"] for result in self.last_results.values()]
        if not outputs[0] == outputs[1] == outputs[2]:
            raise AssertionError("cached/warm/uncached JSON bytes differ")
        self.checkpoints += 1
        if time.monotonic() - self.last_progress >= 30:
            self.check_report_cache()
            self.progress("PASS")

    def check_report_cache(self):
        files = [path for directory in (self.root / "cache").glob("report-*")
                 if directory.is_dir() for path in directory.glob("*.bin")]
        self.peak_report_files = max(self.peak_report_files, len(files))
        self.report_cache = {"files": len(files),
                             "bytes": sum(path.stat().st_size for path in files),
                             "peak_sampled_files": self.peak_report_files}
        limit = self.args.max_report_cache_files
        if limit is not None and len(files) > limit:
            raise AssertionError("report cache contains %d files; limit is %d" % (len(files), limit))

    def progress(self, status):
        self.last_progress = time.monotonic()
        print("%s seed=%d step=%d checks=%d records=%d elapsed=%.1fs" % (
            status, self.args.seed, self.step, self.checkpoints,
            self.last_expected.get("unique_records", 0), self.last_progress - self.started),
            flush=True)

    def writable(self, exclude=None):
        candidates = [source for source in self.sources
                      if source is not exclude and len(source.entries) < MAX_RECORDS_PER_FILE]
        if not candidates:
            source = self.rng.choice([item for item in self.sources if item is not exclude])
            source.entries = source.entries[:MAX_RECORDS_PER_FILE // 2]
            source.write()
            self.operations["capacity_truncate"] += 1
            self.check("capacity_truncate", source)
            return source
        return self.rng.choice(candidates)

    def populated(self):
        candidates = [source for source in self.sources if source.entries]
        if not candidates:
            source = self.writable()
            source.entries.append((self.new_record(), False))
            source.write()
            self.check("reseed_empty_history", source)
            return source
        return self.rng.choice(candidates)

    def mutate(self, operation):
        self.before = {str(source.path.relative_to(self.root)): source.path.read_bytes()
                       for source in self.sources if source.exists}
        self.operations[operation] += 1
        if operation in ("append", "append_whitespace", "partial_completion"):
            source = self.writable()
            record = self.new_record()
            style = operation != "append"
            encoded = record.line(style)
            if operation == "partial_completion":
                # Cut inside the JSON, before any valid closing object.
                cut = self.rng.randrange(1, len(encoded) // 2)
                with source.path.open("ab") as stream:
                    stream.write(encoded[:cut])
                self.check("partial_line", source, cut=cut)
                with source.path.open("ab") as stream:
                    stream.write(encoded[cut:])
            else:
                with source.path.open("ab") as stream:
                    stream.write(encoded)
            source.entries.append((record, style))
            self.check(operation, source)
        elif operation == "duplicate_across_files":
            original = self.populated()
            record, _style = self.rng.choice(original.entries)
            source = self.writable(exclude=original)
            style = bool(self.rng.randrange(2))
            source.entries.append((record, style))
            with source.path.open("ab") as stream:
                stream.write(record.line(style))
            self.check(operation, source, duplicated_id=record.index)
        elif operation in ("rewrite_same_size", "rewrite_larger"):
            source = self.populated() if operation == "rewrite_same_size" else self.writable()
            old_size = source.path.stat().st_size
            source.entries = [(self.new_record(record), style) for record, style in source.entries]
            if operation == "rewrite_larger":
                source.entries.append((self.new_record(), bool(self.rng.randrange(2))))
            source.write()
            new_size = source.path.stat().st_size
            assert (new_size == old_size if operation == "rewrite_same_size" else new_size > old_size)
            self.check(operation, source, old_size=old_size, new_size=new_size)
        elif operation == "truncate":
            source = self.populated()
            keep = self.rng.randrange(len(source.entries))
            source.entries = source.entries[:keep]
            with source.path.open("r+b") as stream:
                stream.truncate(len(source.content()))
            self.check(operation, source, kept_records=keep)
        elif operation == "delete_restore":
            source = self.populated()
            source.path.unlink()
            source.exists = False
            self.check("deleted", source)
            source.write()
            self.check("restored", source)

    def execute(self):
        version = self.run_command(["--version"], "on")
        self.last_results = {"version": version}
        if version["timed_out"] or version["returncode"]:
            raise AssertionError("version command failed (exit=%s, timeout=%s)" % (
                version["returncode"], version["timed_out"]))
        self.version = version["stdout"].decode().strip()
        self.check("empty_history")
        for source in self.sources:
            source.entries = [(self.new_record(), bool(self.rng.randrange(2)))
                              for _ in range(self.rng.randint(1, 3))]
            source.write()
        self.check("seeded_history")
        cycle = []
        deadline = self.started + self.args.duration_seconds if self.args.duration_seconds else None
        while (time.monotonic() < deadline if deadline else self.step < self.args.steps):
            self.step += 1
            if not cycle:
                cycle = list(OPERATIONS)
                self.rng.shuffle(cycle)
            self.mutate(cycle.pop())
            self.steps_completed = self.step
        self.check_report_cache()

    def evidence(self, status):
        return {
            "status": status, "seed": self.args.seed,
            "steps_requested": self.args.steps, "duration_seconds": self.args.duration_seconds,
            "steps_completed": self.steps_completed, "current_step": self.step,
            "checkpoints_passed": self.checkpoints, "child_processes": self.processes,
            "elapsed_seconds": time.monotonic() - self.started,
            "measured_at": datetime.now(timezone.utc).isoformat(),
            "platform": platform.platform(), "python": platform.python_version(),
            "harness_sha256": self.harness_hash,
            "binary": {"path": str(self.args.turbotokens), "sha256": self.binary_hash,
                       "version": self.version},
            "arguments": self.arguments[:-2] + ["--config", "<isolated neutral.json>"],
            "scope": "Synthetic Claude daily reports only; independent current-record oracle for all four token categories and recorded cost; cached/warm/uncached byte parity. No live, daemon, or performance claims.",
            "max_records_per_file": MAX_RECORDS_PER_FILE,
            "report_cache": self.report_cache,
            "max_report_cache_files": self.args.max_report_cache_files,
            "operations": dict(self.operations), "recent_checkpoints": list(self.recent),
            "last_oracle": self.last_expected,
        }

    def preserve_failure(self, output):
        destination = Path(tempfile.mkdtemp(
            prefix=output.stem + "-failure-", dir=output.parent))
        shutil.copytree(self.root, destination / "snapshot")
        for name, content in self.before.items():
            path = destination / "before" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        (destination / "oracle.json").write_text(json.dumps(self.last_expected, indent=2) + "\n")
        results = {}
        for label, result in self.last_results.items():
            (destination / (label + ".stdout")).write_bytes(result["stdout"])
            (destination / (label + ".stderr")).write_bytes(result["stderr"])
            results[label] = {key: value for key, value in result.items()
                              if key not in ("stdout", "stderr")}
        (destination / "processes.json").write_text(json.dumps(results, indent=2) + "\n")
        return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--turbotokens", type=Path, required=True,
                        help="Absolute path to an installed native executable")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--steps", type=int, default=100)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--duration-seconds", type=float,
                        help="Run for this duration instead of stopping at --steps")
    parser.add_argument("--max-report-cache-files", type=int,
                        help="Fail if report-cache files exceed this count; sampled every 30 seconds and at completion")
    args = parser.parse_args()
    if not args.turbotokens.is_absolute() or not args.turbotokens.is_file():
        parser.error("--turbotokens must name an existing absolute executable path")
    if not os.access(args.turbotokens, os.X_OK):
        parser.error("--turbotokens must be executable")
    if args.steps < 1:
        parser.error("--steps must be positive")
    if args.max_report_cache_files is not None and args.max_report_cache_files < 1:
        parser.error("--max-report-cache-files must be positive")
    if args.duration_seconds is not None and (
            not math.isfinite(args.duration_seconds) or args.duration_seconds <= 0):
        parser.error("--duration-seconds must be finite and positive")
    args.turbotokens = args.turbotokens.resolve()
    args.output = args.output.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def interrupted(signum, _frame):
        raise KeyboardInterrupt("signal %s" % signum)

    signal.signal(signal.SIGTERM, interrupted)
    exit_code = 0
    with tempfile.TemporaryDirectory(prefix="turbotokens-stress-") as temporary:
        run = StressRun(args, Path(temporary))
        try:
            run.execute()
            result = run.evidence("passed")
        except (Exception, KeyboardInterrupt) as error:
            interrupted = isinstance(error, KeyboardInterrupt)
            exit_code = 130 if interrupted else 1
            result = run.evidence("interrupted" if interrupted else "failed")
            result.update(error_type=type(error).__name__, error=str(error),
                          traceback=traceback.format_exc())
            failure = run.preserve_failure(args.output)
            result["failure_directory"] = str(failure)
            result["reproduce"] = [sys.executable, str(failure / "snapshot" / run.harness.name),
                "--turbotokens", str(failure / "snapshot" / run.binary.name),
                "--seed", str(args.seed), "--steps", str(max(1, run.step)),
                "--output", str(failure / "reproduction.json")]
            if args.max_report_cache_files is not None:
                result["reproduce"] += ["--max-report-cache-files", str(args.max_report_cache_files)]
            (failure / "failure.json").write_text(json.dumps(result, indent=2) + "\n")
            print("Saved failing seed/step, fixtures, binary, and output: " + str(failure),
                  file=sys.stderr, flush=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        run.progress(result["status"].upper())
    print("Evidence: " + str(args.output), flush=True)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
