// Install the packed npm artifact in isolation, then exercise its actual npm shim.
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const runtime = process.env.TURBOTOKENS_TEST_NODE || process.execPath;
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
let environment = process.env;
const run = (command, args, options = {}) => execFileSync(command, args, {
  encoding: 'utf8', shell: process.platform === 'win32', env: environment, ...options,
});
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'turbotokens-npm-'));
try {
  if (process.platform === 'win32') {
    // Prove installation ignores a conflicting tar.exe earlier on PATH.
    const shadow = path.join(temporary, 'shadow bin');
    fs.mkdirSync(shadow);
    fs.copyFileSync(process.execPath, path.join(shadow, 'tar.exe'));
    environment = { ...process.env };
    const pathKey = Object.keys(environment).find(key => key.toLowerCase() === 'path') || 'Path';
    environment[pathKey] = shadow + path.delimiter + (environment[pathKey] || '');
    assert.equal(run('tar', ['--version']).trim(), process.version);
  }
  const packed = JSON.parse(run(npm, ['pack', path.join(root, 'npm'), '--json'], { cwd: temporary }))[0];
  assert(packed.files.some(file => file.path === 'bin/turbotokens.js'));
  assert(packed.files.some(file => file.path === 'LICENSE'));
  assert(!packed.files.some(file => file.path.startsWith('vendor/')));
  // npm 6 treats --prefix as a global install on Windows even with --global=false.
  // Use a real project directory so every supported npm version creates a local shim.
  fs.writeFileSync(path.join(temporary, 'package.json'), JSON.stringify({ name: 'turbotokens-smoke', private: true }));
  run(npm, ['install', '--global=false', '--ignore-scripts=false', '--no-audit', '--no-fund', path.join(temporary, packed.filename)], { cwd: temporary });
  const executable = path.join(temporary, 'node_modules', '.bin', process.platform === 'win32' ? 'turbotokens.cmd' : 'turbotokens');
  const version = require('../npm/package.json').version;
  assert.equal(run(executable, ['--version']).trim(), `turbotokens ${version}`);
  // The shim downloads the published GitHub release, not this checkout.
  // Gate unreleased commands so main stays green between CLI work and a tag.
  const rootHelp = run(executable, ['--help']);
  if (/^\s+stream\s/m.test(rootHelp)) {
    assert.match(run(executable, ['stream', '--help']), /newline-delimited JSON/);
  }
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
  assert.throws(() => run(runtime, [path.join(installed, 'install.js')]), error => error.status === 1 && /checksum mismatch/.test(error.stderr));
  assert(!fs.existsSync(path.join(installed, 'vendor')));
  console.log(`npm package passed: ${process.platform}-${process.arch}, v${version}`);
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
