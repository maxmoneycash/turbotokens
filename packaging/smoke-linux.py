#!/usr/bin/env python3
"""Reject dynamically linked Linux builds and exercise reports across libc families."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(*args):
    return subprocess.check_output(args, text=True, timeout=180).strip()


def check_linkage(binary):
    headers = run('readelf', '--program-headers', str(binary))
    dynamic = run('readelf', '--dynamic', str(binary))
    if 'INTERP' in headers or '(NEEDED)' in dynamic:
        raise RuntimeError('Linux release must have no interpreter or shared-library dependencies')
    print('ELF linkage passed: no interpreter or shared-library dependencies', flush=True)


def check_reports(binary, version):
    with tempfile.TemporaryDirectory(prefix='turbotokens-linux-') as temporary:
        fixture = Path(temporary)
        project = fixture / 'projects' / 'smoke'
        project.mkdir(parents=True)
        event = {
            'type': 'assistant',
            'timestamp': '2026-01-02T12:00:00.000Z',
            'requestId': 'req-linux-smoke',
            'message': {
                'id': 'msg-linux-smoke',
                'model': 'claude-sonnet-4-20250514',
                'usage': {
                    'input_tokens': 100,
                    'output_tokens': 200,
                    'cache_creation_input_tokens': 30,
                    'cache_read_input_tokens': 40,
                },
            },
        }
        # A duplicate event must be counted only once in every target environment.
        (project / 'session.jsonl').write_text((json.dumps(event, separators=(',', ':')) + '\n') * 2)
        expected = {
            'inputTokens': 100,
            'outputTokens': 200,
            'cacheCreationTokens': 30,
            'cacheReadTokens': 40,
            'totalTokens': 370,
        }
        reference = None
        for image in ('ubuntu:22.04', 'debian:bookworm-slim', 'alpine:3.22'):
            command = [
                'docker', 'run', '--rm', '--network', 'none', '--read-only',
                '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                '--user', f'{os.getuid()}:{os.getgid()}', '--tmpfs', '/tmp:rw,mode=1777',
                '--volume', f'{binary}:/usr/local/bin/turbotokens:ro',
                '--volume', f'{fixture}:/fixture:ro',
                '--env', 'CLAUDE_CONFIG_DIR=/fixture', '--env', 'HOME=/tmp',
                '--env', 'XDG_CACHE_HOME=/tmp/.cache', '--env', 'TZ=UTC', image,
            ]
            actual_version = run(*command, 'turbotokens', '--version')
            if actual_version != f'turbotokens {version}':
                raise RuntimeError(f'{image}: unexpected version {actual_version!r}')
            report = run(*command, 'turbotokens', 'claude', 'daily', '--json', '--offline', '--timezone', 'UTC')
            data = json.loads(report)
            for field, value in expected.items():
                if data['totals'][field] != value:
                    raise RuntimeError(f'{image}: {field} expected {value}, got {data["totals"][field]}')
            if len(data['daily']) != 1 or data['daily'][0]['date'] != '2026-01-02':
                raise RuntimeError(f'{image}: unexpected daily grouping')
            if data['totals']['totalCost'] <= 0:
                raise RuntimeError(f'{image}: embedded pricing produced no cost')
            if reference is not None and report != reference:
                raise RuntimeError(f'{image}: report differs from Ubuntu JSON')
            reference = report
            print(f'{image}: exact version, deduped token totals, embedded pricing, matching JSON passed', flush=True)


if __name__ == '__main__':
    if len(sys.argv) != 3:
        raise SystemExit('Usage: python3 packaging/smoke-linux.py <binary> <version>')
    release_binary = Path(sys.argv[1]).resolve(strict=True)
    check_linkage(release_binary)
    check_reports(release_binary, sys.argv[2])
