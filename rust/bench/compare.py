#!/usr/bin/env python3
"""Compare native monthly JSON reports on identical synthetic Claude logs.

All tools get a warm-up before interleaved measurements. No package manager
or network pricing warm-up is included. Checks all four token categories on
every run; pricing and JSON schemas are intentionally not compared.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time


def counters(tool, output):
    report = json.loads(output)
    if tool == 'tokscale':
        return [sum(row[key] for row in report['entries'])
                for key in ('input', 'output', 'cacheWrite', 'cacheRead')]
    return [report['totals'][key] for key in
            ('inputTokens', 'outputTokens', 'cacheCreationTokens', 'cacheReadTokens')]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for tool in ('turbotokens', 'ccusage', 'tokscale'):
        parser.add_argument('--' + tool, type=Path, required=True)
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sizes', nargs='+', type=int, default=[1, 5, 10, 25, 50])
    parser.add_argument('--runs', type=int, default=5)
    parser.add_argument('--report', choices=('daily', 'monthly'), default='monthly')
    args = parser.parse_args()
    if args.runs < 3:
        parser.error('Use at least three runs')
    names = ('turbotokens', 'ccusage', 'tokscale') if args.report == 'monthly' else ('turbotokens', 'ccusage')
    binaries = {name: getattr(args, name).resolve() for name in names}
    tools = {name: {
        'version': subprocess.check_output([str(path), '--version'], text=True).strip(),
        'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
        'file': subprocess.check_output(['file', '-b', str(path)], text=True).strip(),
    } for name, path in binaries.items()}
    rows = []
    with tempfile.TemporaryDirectory(prefix='turbotokens-compare-') as scratch:
        root = Path(scratch)
        for size in args.sizes:
            data = (args.data / ('tok-%sB' % size)).resolve()
            home = root / str(size)
            home.mkdir()
            (home / '.claude').symlink_to(data, target_is_directory=True)
            env = dict(os.environ, CLAUDE_CONFIG_DIR=str(data),
                       TURBOTOKENS_CACHE_DIR=str(home / 'tt-cache'),
                       TURBOTOKENS_CACHE='on',
                       TOKSCALE_CONFIG_DIR=str(root / 'tokscale-config'),
                       TZ='UTC', NO_COLOR='1')
            commands = {
                name: [str(binaries[name]), 'claude', args.report, '--offline', '--json']
                for name in ('turbotokens', 'ccusage')
            }
            if 'tokscale' in names:
                commands['tokscale'] = [str(binaries['tokscale']), 'monthly', '--client',
                                        'claude', '--home', str(home), '--json', '--no-spinner']
            expected = None

            def run(name):
                start = time.perf_counter()
                result = subprocess.run(commands[name], env=env, check=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        timeout=300)
                elapsed = time.perf_counter() - start
                counts = counters(name, result.stdout)
                if sum(counts) != int((data / 'TOTAL_TOKENS').read_text()):
                    raise RuntimeError('%s total differs from fixture' % name)
                if expected is not None and counts != expected:
                    raise RuntimeError('%s token categories differ: %r != %r' %
                                       (name, counts, expected))
                return elapsed, counts

            for name in names:
                _, expected = run(name)
            samples = {name: [] for name in names}
            for repetition in range(args.runs):
                # Rotate the first tool to reduce systematic order effects.
                offset = repetition % len(names)
                order = names[offset:] + names[:offset]
                for name in order:
                    elapsed, _ = run(name)
                    samples[name].append(elapsed)
                    print('%sB / %s / run %s: %.3fs' %
                          (size, name, repetition + 1, elapsed), flush=True)
            rows.append({'recorded_tokens': sum(expected),
                         'bytes': sum(path.stat().st_size for path in data.rglob('*.jsonl')),
                         'files': len(list(data.rglob('*.jsonl'))),
                         'token_categories': dict(zip(('input', 'output', 'cache_creation', 'cache_read'), expected)),
                         'token_parity': True, 'seconds': samples,
                         'median_seconds': {name: statistics.median(times) for name, times in samples.items()}})
    result = {'measured_at': datetime.now(timezone.utc).isoformat(),
              'host': platform.platform(),
              'processor': subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string'], text=True).strip()
              if platform.system() == 'Darwin' else platform.processor(),
              'tools': tools, 'runs': args.runs,
              'report': args.report,
              'method': 'Native JSON reports, warm application and OS caches, rotating tool order, subprocess wall time including output. Pricing is primed before timing. Four token categories match on every run. Identical synthetic source files per tool; file count is recorded per dataset. No daemon.',
              'commands': {name: command[1:] for name, command in commands.items()},
              'datasets': rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
