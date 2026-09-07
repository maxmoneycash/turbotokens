// Install the packed npm artifact in isolation, then exercise its actual npm shim.
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const run = (command, args, options = {}) => execFileSync(command, args, {
  encoding: 'utf8', shell: process.platform === 'win32', ...options,
});
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'turbotokens-npm-'));
try {
  const packed = JSON.parse(run(npm, ['pack', '--json', '--pack-destination', temporary], { cwd: path.join(root, 'npm') }))[0];
  assert(packed.files.some(file => file.path === 'bin/turbotokens.js'));
  assert(packed.files.some(file => file.path === 'LICENSE'));
  assert(!packed.files.some(file => file.path.startsWith('vendor/')));
  run(npm, ['install', '--prefix', temporary, '--ignore-scripts=false', '--no-audit', '--no-fund', path.join(temporary, packed.filename)]);
  const executable = path.join(temporary, 'node_modules', '.bin', process.platform === 'win32' ? 'turbotokens.cmd' : 'turbotokens');
  const version = require('../npm/package.json').version;
  assert.equal(run(executable, ['--version']).trim(), `turbotokens ${version}`);
  assert.match(run(executable, ['stream', '--help']), /newline-delimited JSON/);
  assert.match(run(executable, ['heatmap', '--help']), /--svg/);
  assert.match(run(executable, ['wrapped', '--help']), /--year/);
  assert.match(run(executable, ['limits', '--help']), /plan-limit/);
  assert.match(run(executable, ['completions', '--help']), /bash\|zsh\|fish/);
  const installed = path.join(temporary, 'node_modules', 'turbotokens');
  fs.rmSync(path.join(installed, 'vendor'), { recursive: true });
  assert.throws(() => run(executable, ['--version']), error => error.status === 1 && /binary is missing/.test(error.stderr));
  // A corrupt checksum must fail before a binary is installed.
  const sumsPath = path.join(installed, 'checksums.json');
  const sums = JSON.parse(fs.readFileSync(sumsPath));
  for (const asset of Object.keys(sums)) sums[asset] = '0'.repeat(64);
  fs.writeFileSync(sumsPath, JSON.stringify(sums));
  assert.throws(() => run(process.execPath, [path.join(installed, 'install.js')]), error => error.status === 1 && /checksum mismatch/.test(error.stderr));
  assert(!fs.existsSync(path.join(installed, 'vendor')));
  console.log(`npm package passed: ${process.platform}-${process.arch}, v${version}`);
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
