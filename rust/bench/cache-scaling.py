#!/usr/bin/env python3
"""Measure uncached and cached reports on deterministic synthetic Claude logs.

Usage: python3 rust/bench/cache-scaling.py --data /tmp/tt-cache-scaling \
    --binary rust/target/release/turbotokens --output rust/bench/results/cache-scaling.json
The process timer includes JSON output. Every result must match byte for byte.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sizes', nargs='+', type=int, default=[1, 5, 10, 25, 50])
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    if args.runs < 1 or any(size < 1 for size in args.sizes):
        parser.error('runs and sizes must be positive')
    binary = args.binary.resolve()
    args.data.mkdir(parents=True, exist_ok=True)
    command = [str(binary), 'claude', 'daily', '--offline', '--json']
    rows = []
    for size in args.sizes:
        directory = args.data.resolve() / f'tok-{size}B'
        if not (directory / 'TOTAL_TOKENS').exists():
            subprocess.run([sys.executable, str(Path(__file__).with_name('gen_scaling_data.py')), str(directory), str(size)], check=True)
        env = dict(os.environ, CLAUDE_CONFIG_DIR=str(directory), TURBOTOKENS_CACHE_DIR=str(directory / 'cache'), TZ='UTC', NO_COLOR='1')
        expected_tokens = int((directory / 'TOTAL_TOKENS').read_text())
        samples = {'uncached': [], 'cached': []}
        reference = None

        def run(mode):
            start = time.perf_counter()
            output = subprocess.run(command, env=dict(env, TURBOTOKENS_CACHE='off' if mode == 'uncached' else 'on'), stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout
            elapsed = time.perf_counter() - start
            if json.loads(output)['totals']['totalTokens'] != expected_tokens:
                raise RuntimeError(f'token count mismatch for {size}B')
            return elapsed, output

        # Prime OS file cache and the application cache, outside the measurements.
        _, reference = run('uncached')
        if run('cached')[1] != reference:
            raise RuntimeError('cache warm-up changed JSON output')
        for _ in range(args.runs):
            for mode in samples:
                elapsed, output = run(mode)
                if output != reference:
                    raise RuntimeError(f'{mode} report changed JSON output')
                samples[mode].append(elapsed)
        row = {'tokens': expected_tokens, 'bytes': sum(p.stat().st_size for p in directory.rglob('*.jsonl')), 'files': len(list(directory.rglob('*.jsonl'))), 'seconds': samples, 'median_seconds': {mode: statistics.median(values) for mode, values in samples.items()}, 'byte_parity': True}
        rows.append(row)
        print(f'{size}B: {row["median_seconds"]} (byte parity passed)', flush=True)
    result = {'measured_at': datetime.now(timezone.utc).isoformat(), 'platform': platform.platform(), 'processor': subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string'], text=True).strip() if sys.platform == 'darwin' else platform.processor(), 'version': subprocess.check_output([str(binary), '--version'], text=True).strip(), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'runs': args.runs, 'method': 'Alternating uncached/cached subprocesses; median wall time including JSON stdout; warm OS file cache; offline pricing; six synthetic JSONL files per dataset; exact JSON parity on every run.', 'datasets': rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
