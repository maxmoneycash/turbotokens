#!/usr/bin/env python3
"""Redistribute a generated dataset across N files without changing its records.

Usage: shard_data.py input_directory new_output_directory file_count
The output must not exist, preventing accidental duplicate records on a rerun.
"""
from pathlib import Path
import sys

source, destination, count = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
if count < 1:
    raise SystemExit('file_count must be positive')
destination.mkdir(parents=True, exist_ok=False)
project = destination / 'projects/benchmark'
project.mkdir(parents=True)
index = 0
for path in sorted(source.glob('projects/**/*.jsonl')):
    with path.open('rb') as stream:
        for line in stream:
            # Open one output at a time to work under low file-descriptor limits.
            with (project / ('session-%04d.jsonl' % (index % count))).open('ab') as target:
                target.write(line)
            index += 1
(destination / 'TOTAL_TOKENS').write_bytes((source / 'TOTAL_TOKENS').read_bytes())
print('%s records across %s files' % (index, min(index, count)))
