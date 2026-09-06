#!/usr/bin/env python3
"""Capture real CLI output using isolated, deterministic synthetic agent logs.

Usage: python3 demo/showcase.py --binary rust/target/release/turbotokens
The child gets a minimal environment, so personal agent histories are never
included. USERPROFILE is the CLI's fallback home; the parent environment is
unchanged. Live events use the current clock and are recorded at actual speed.
"""
import argparse
from datetime import datetime, timedelta, timezone
import fcntl
import json
import os
from pathlib import Path
import pty
import random
import select
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[1]


def claude_event(timestamp, index, project, rng):
    return {'type': 'assistant', 'timestamp': timestamp, 'version': '2.0.0',
            'sessionId': project + '-session', 'requestId': 'request-' + str(index),
            'message': {'id': 'message-' + str(index),
                        'model': 'claude-sonnet-4-20250514', 'role': 'assistant',
                        'content': [{'type': 'text', 'text': 'Synthetic demo response.'}],
                        'usage': {'input_tokens': rng.randint(300, 1600),
                                  'output_tokens': rng.randint(200, 900),
                                  'cache_creation_input_tokens': rng.randint(0, 4000),
                                  'cache_read_input_tokens': rng.randint(4000, 75000)}}}


def append(path, event):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('a') as stream:
        stream.write(json.dumps(event, separators=(',', ':')) + '\n')


def seed(root):
    rng = random.Random(42)
    today = datetime.now(timezone.utc).replace(hour=9, minute=0, second=0, microsecond=0)
    index = 0
    for offset in range(249):
        day = today - timedelta(days=248-offset)
        if day.weekday() >= 5 and rng.random() < .65 and offset < 246:
            continue
        for project in ('webapp', 'api', 'infra'):
            path = root / '.claude/projects' / project / (project + '-session.jsonl')
            for _ in range(rng.randint(2, 15)):
                index += 1
                append(path, claude_event(day.isoformat(), index, project, rng))
        codex_path = root / '.codex/sessions' / ('demo-%s.jsonl' % offset)
        append(codex_path, {'type': 'session_meta', 'timestamp': day.isoformat(),
                           'payload': {'id': 'codex-%s' % offset, 'cwd': '/demo/api'}})
        append(codex_path, {'type': 'turn_context', 'timestamp': day.isoformat(),
                           'payload': {'model': 'gpt-5'}})
        usage = {'input_tokens': rng.randint(60000, 220000),
                 'cached_input_tokens': 40000, 'output_tokens': rng.randint(10000, 40000),
                 'reasoning_output_tokens': 0}
        usage['total_tokens'] = usage['input_tokens'] + usage['output_tokens']
        append(codex_path, {'type': 'event_msg', 'timestamp': day.isoformat(),
                           'payload': {'type': 'token_count', 'info': {'total_token_usage': usage,
                                                                    'last_token_usage': usage}}})
    return today


def record_live(binary, env, root, destination):
    # A short, active work session makes the live view legible; the yearly
    # exports above use the separate long-history fixture.
    shutil.rmtree(root / '.claude/projects')
    now = datetime.now(timezone.utc)
    live_rng = random.Random(81)
    for index in range(60):
        project = ('webapp', 'api', 'infra')[index % 3]
        event = claude_event((now - timedelta(seconds=(60-index)*35)).isoformat(timespec='milliseconds'),
                             200000 + index, project, live_rng)
        if project == 'infra':
            event['message']['model'] = 'claude-opus-4-1-20250805'
        append(root / '.claude/projects' / project / (project + '-session.jsonl'), event)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 104, 0, 0))
    live_env = dict(env, TERM='xterm-256color', FORCE_COLOR='1', COLUMNS='104')
    live_env.pop('NO_COLOR', None)
    child = subprocess.Popen([str(binary), 'live', '--offline'], env=live_env,
                             stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    os.close(slave)
    rng = random.Random(81)
    start, next_event, index = time.monotonic(), 2.0, 100000
    try:
        with destination.open('w') as cast:
            cast.write(json.dumps({'version': 2, 'width': 104, 'height': 40,
                                   'title': 'turbotokens live · synthetic usage',
                                   'env': {'TERM': 'xterm-256color'}}) + '\n')
            while time.monotonic() - start < 12:
                elapsed = time.monotonic() - start
                if elapsed >= next_event:
                    index += 1
                    project = ('webapp', 'api', 'infra')[index % 3]
                    event = claude_event(datetime.now(timezone.utc).isoformat(timespec='milliseconds'), index, project, rng)
                    if project == 'infra':
                        event['message']['model'] = 'claude-opus-4-1-20250805'
                    append(root / '.claude/projects' / project / (project + '-session.jsonl'), event)
                    next_event += .65
                if select.select([master], [], [], .05)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    cast.write(json.dumps([time.monotonic()-start, 'o', chunk.decode('utf-8', errors='replace')]) + '\n')
            if child.poll() is not None:
                raise RuntimeError('Live dashboard exited before recording finished')
    finally:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGINT)
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        os.close(master)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    fixtures = ROOT / 'demo/fixtures'
    with tempfile.TemporaryDirectory(prefix='tt-demo-', dir='/tmp') as temporary:
        root = Path(temporary)
        today = seed(root)
        env = {'PATH': os.environ['PATH'], 'USERPROFILE': str(root),
               'CLAUDE_CONFIG_DIR': str(root / '.claude'), 'TZ': 'UTC', 'NO_COLOR': '1', 'COLUMNS': '150',
               'TURBOTOKENS_CACHE_DIR': str(root / 'cache')}
        commands = {
            'daily.txt': ['daily', '--last', '3', '--offline'],
            'daily.json': ['daily', '--last', '3', '--offline', '--json'],
            'heatmap.svg': ['heatmap', '--offline', '--svg', str(ROOT / 'assets/heatmap.svg'), '--json'],
            'wrapped.svg': ['wrapped', '--year', str(today.year), '--offline', '--svg', str(ROOT / 'assets/wrapped.svg'), '--json'],
        }
        for filename, command in commands.items():
            output = subprocess.check_output([str(binary)] + command, env=env)
            if filename.endswith('.svg'):
                (fixtures / filename).write_bytes((ROOT / 'assets' / filename).read_bytes())
                (fixtures / filename.replace('.svg', '.json')).write_bytes(output)
            else:
                (fixtures / filename).write_bytes(output)
        record_live(binary, env, root, fixtures / 'live.cast')
        (fixtures / 'capture.json').write_text(json.dumps({
            'version': subprocess.check_output([str(binary), '--version'], text=True).strip(),
            'captured_at': datetime.now(timezone.utc).isoformat(), 'data': 'Synthetic, seed 42',
            'commands': commands, 'live': 'live --offline; 12 seconds, original speed'}, indent=2) + '\n')


if __name__ == '__main__':
    main()
