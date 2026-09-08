#!/usr/bin/env python3
"""Check a native release using isolated synthetic logs (Python 3.9+, stdlib only)."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import socket
import subprocess
import sys
import tempfile
import time


COUNTS = ("inputTokens", "outputTokens", "cacheCreationTokens", "cacheReadTokens")
RAW_COUNTS = ("input_tokens", "output_tokens", "cache_creation_input_tokens",
              "cache_read_input_tokens")
DATE = "2026-01-02"


def record(counts, whitespace=False, date=DATE):
    event = {
        "type": "assistant", "timestamp": date + "T12:00:00.000Z",
        "sessionId": "native-smoke", "requestId": "req-native-smoke", "costUSD": 0.125,
        "message": {"id": "msg-native-smoke", "role": "assistant",
                    "model": "claude-sonnet-4-20250514",
                    "usage": dict(zip(RAW_COUNTS, counts))},
    }
    separators = (", ", ": ") if whitespace else (",", ":")
    padding = " \t" if whitespace else ""
    return (padding + json.dumps(event, separators=separators) + padding + "\n").encode()


def isolated_environment(root):
    directories = {name: root / name for name in ("home", "cache", "config", "tmp", "codex")}
    for directory in directories.values():
        directory.mkdir()
    env = {
        "PATH": os.defpath, "HOME": str(directories["home"]),
        "USERPROFILE": str(directories["home"]),
        "CLAUDE_CONFIG_DIR": str(directories["home"] / ".claude"),
        "CODEX_HOME": str(directories["codex"]),
        "XDG_CONFIG_HOME": str(directories["config"]),
        "XDG_CACHE_HOME": str(directories["cache"]),
        "APPDATA": str(directories["config"]), "LOCALAPPDATA": str(directories["cache"]),
        "TURBOTOKENS_CACHE_DIR": str(directories["cache"]), "TURBOTOKENS_CACHE": "on",
        "TMPDIR": str(directories["tmp"]), "TEMP": str(directories["tmp"]),
        "TMP": str(directories["tmp"]), "TZ": "UTC", "NO_COLOR": "1",
        "LOG_LEVEL": "0", "PYTHONNOUSERSITE": "1",
    }
    for key in ("SYSTEMROOT", "WINDIR", "COMSPEC"):
        if key in os.environ:
            env[key] = os.environ[key]
    return env


def process_evidence(child):
    return {"exit_code": child.returncode,
            "stdout": child.stdout.decode("utf-8", errors="replace"),
            "stderr": child.stderr.decode("utf-8", errors="replace")}


def validate_report(raw, counts):
    value = json.loads(raw)
    expected = dict(zip(COUNTS, counts), totalTokens=sum(counts), totalCost=0.125)
    if len(value["daily"]) != 1 or value["daily"][0]["date"] != DATE:
        raise AssertionError("expected one daily bucket for " + DATE)
    for label, row in (("daily", value["daily"][0]), ("totals", value["totals"])):
        for key, wanted in expected.items():
            if row[key] != wanted:
                raise AssertionError("%s.%s: expected %r, got %r" % (label, key, wanted, row[key]))


def stamp(path):
    metadata = path.stat()
    return {"size": metadata.st_size, "mtime_ns": metadata.st_mtime_ns,
            "ctime_ns": metadata.st_ctime_ns, "inode": metadata.st_ino}


def live_rewrites(binary, root, env, source, config):
    counts = (100, 200, 30, 40)
    today = datetime.now(timezone.utc).date().isoformat()
    initial = record(counts, date=today)
    source.write_bytes(initial)
    os.utime(source, ns=(1_700_000_000_000_000_000,) * 2)
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        address = listener.getsockname()
    command = [str(binary), "live", "--json", "--interval", "20", "--serve",
               "%s:%s" % address, "--offline", "--mode", "display", "--timezone", "UTC",
               "--config", str(config)]
    phases = []
    with (root / "live.stdout").open("wb") as stdout, (root / "live.stderr").open("wb") as stderr:
        child = subprocess.Popen(command, cwd=env["HOME"], env=env,
                                 stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        def wait_counts(name, wanted):
            deadline = time.monotonic() + 10
            last = "no response"
            while time.monotonic() < deadline:
                if child.poll() is not None:
                    raise AssertionError("live exited with %s" % child.returncode)
                try:
                    with socket.create_connection(address, timeout=1) as connection:
                        body = b""
                        while True:
                            part = connection.recv(65536)
                            if not part:
                                break
                            body += part
                    text = body.decode("utf-8")
                    actual = {}
                    for line in text.splitlines():
                        for kind in ("input", "output", "cache_creation", "cache_read"):
                            prefix = 'turbotokens_tokens_total{kind="%s"} ' % kind
                            if line.startswith(prefix):
                                actual[kind] = int(line[len(prefix):])
                    last = actual
                    if tuple(actual.get(kind) for kind in ("input", "output", "cache_creation", "cache_read")) == wanted:
                        phases.append({"phase": name, "counts": actual})
                        return
                except (OSError, ValueError) as error:
                    last = str(error)
                time.sleep(0.02)
            raise AssertionError("live %s did not reach %r: %r" % (name, wanted, last))
        try:
            wait_counts("seed", counts)
            before = source.stat()
            rewritten = record((101, 201, 31, 41), date=today)
            assert len(rewritten) == before.st_size
            with source.open("r+b") as stream:
                stream.write(rewritten)
            os.utime(source, ns=(before.st_atime_ns, before.st_mtime_ns))
            assert source.stat().st_mtime_ns == before.st_mtime_ns
            wait_counts("in_place_preserved_mtime", (101, 201, 31, 41))
            source.unlink()
            wait_counts("deleted", (0, 0, 0, 0))
            source.write_bytes(rewritten)
            wait_counts("restored", (101, 201, 31, 41))
        finally:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
    return {"passed": True, "phases": phases, "owned_process_stopped": child.poll() is not None}


def smoke(binary, output, root, evidence):
    env = isolated_environment(root)
    home = Path(env["HOME"])
    source = home / ".claude" / "projects" / "smoke" / "native-smoke.jsonl"
    source.parent.mkdir(parents=True)
    config = root / "config" / "neutral.json"
    config.write_text("{}\n", encoding="utf-8")
    arguments = ["claude", "daily", "--json", "--offline", "--mode", "display",
                 "--timezone", "UTC", "--config", str(config)]

    def run(args, cache="on"):
        return subprocess.run([str(binary), *args], cwd=home,
                              env=dict(env, TURBOTOKENS_CACHE=cache),
                              capture_output=True, timeout=30)

    def reports(name, counts, metadata=None):
        # Capture all three even on a mismatch, distinguishing stale caches
        # from parser errors in the uploaded evidence.
        children = {label: run(arguments, cache) for label, cache in
                    (("cached", "on"), ("warm", "on"), ("uncached", "off"))}
        errors = []
        for label, child in children.items():
            try:
                if child.returncode:
                    raise AssertionError("exit code %s" % child.returncode)
                validate_report(child.stdout, counts)
            except (AssertionError, ValueError, KeyError, TypeError) as error:
                errors.append("%s: %s" % (label, error))
        same_bytes = len({child.stdout for child in children.values()}) == 1
        if not same_bytes:
            errors.append("cached/warm/uncached JSON bytes differ")
        evidence["checks"][name] = {
            "passed": not errors, "expected_counts": dict(zip(COUNTS, counts)),
            "byte_identical": same_bytes, "errors": errors, "metadata": metadata,
            "processes": {label: process_evidence(child) for label, child in children.items()},
        }

    version = run(["--version"])
    evidence["version"] = process_evidence(version)
    if version.returncode:
        raise AssertionError("native --version failed")

    counts = (100, 200, 30, 40)
    source.write_bytes(record(counts))
    reports("first_report", counts)
    cache_files = [path.relative_to(root / "cache").as_posix()
                   for path in (root / "cache").rglob("*.bin")]
    evidence["checks"]["cache_created"] = {
        "passed": any(path.startswith("parse-v1/") for path in cache_files)
                  and any(path.startswith("report-v1/") for path in cache_files),
        "files": sorted(cache_files),
    }
    source.write_bytes(record(counts, whitespace=True))
    reports("whitespace_json", counts)

    broken = root / "config" / "broken.json"
    broken.write_text('{"timezone":', encoding="utf-8")
    bad_config = run(arguments[:-1] + [str(broken)])
    evidence["checks"]["explicit_bad_config"] = {
        "passed": bad_config.returncode != 0 and bad_config.stdout == b"",
        **process_evidence(bad_config),
    }

    source.write_bytes(record(counts))
    # Use whole seconds before priming the cache: some Python/OS combinations
    # round os.utime nanoseconds even when stat exposes finer precision.
    preserved_time_ns = 1_700_000_000_000_000_000
    os.utime(source, ns=(preserved_time_ns, preserved_time_ns))
    reports("rewrite_baseline", counts)
    for operation, new_counts in (("atomic_replacement", (101, 201, 31, 41)),
                                  ("in_place_rewrite", (102, 202, 32, 42))):
        previous = source.stat()
        before = stamp(source)
        content = record(new_counts)
        if len(content) != previous.st_size:
            raise AssertionError("rewrite fixture must keep the same byte length")
        if operation == "atomic_replacement":
            replacement = source.with_suffix(".replacement")
            replacement.write_bytes(content)
            os.utime(replacement, ns=(previous.st_atime_ns, previous.st_mtime_ns))
            os.replace(replacement, source)
        else:
            with source.open("r+b") as stream:
                stream.write(content)
            os.utime(source, ns=(previous.st_atime_ns, previous.st_mtime_ns))
        after = stamp(source)
        if after["size"] != before["size"] or after["mtime_ns"] != before["mtime_ns"]:
            raise AssertionError(operation + ": size/mtime preservation failed")
        reports(operation + "_preserved_mtime", new_counts, {"before": before, "after": after})

    try:
        evidence["checks"]["live_rewrites"] = live_rewrites(binary, root, env, source, config)
    except (AssertionError, OSError, ValueError) as error:
        evidence["checks"]["live_rewrites"] = {"passed": False, "error": str(error)}

    stress = Path(__file__).resolve().parents[1] / "rust" / "bench" / "stress-claude.py"
    stress_output = output.with_name(output.stem + "-stress.json")
    child = subprocess.run(
        [sys.executable, str(stress), "--turbotokens", str(binary), "--steps", "32",
         "--seed", "20260908", "--output", str(stress_output)],
        cwd=home, env=env, capture_output=True, timeout=180,
    )
    stress_evidence = {}
    if stress_output.is_file():
        stress_evidence = json.loads(stress_output.read_text(encoding="utf-8"))
    evidence["checks"]["stress_claude"] = {
        "passed": child.returncode == 0 and stress_evidence.get("status") == "passed"
                  and stress_evidence.get("steps_completed") == 32,
        "process": process_evidence(child), "evidence": stress_evidence,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, default=Path("native-smoke.json"))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    evidence = {"status": "failed", "platform": platform.platform(),
                "python": platform.python_version(), "binary": str(binary),
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "checks": {}}
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="turbotokens-native-") as temporary:
            smoke(binary, output, Path(temporary), evidence)
        if all(check["passed"] for check in evidence["checks"].values()):
            evidence["status"] = "passed"
    except Exception as error:
        evidence["error"] = "%s: %s" % (type(error).__name__, error)
    evidence["elapsed_seconds"] = time.monotonic() - started
    output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": evidence["status"], "evidence": str(output),
                      "checks": {name: check["passed"] for name, check in evidence["checks"].items()}}))
    return 0 if evidence["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
