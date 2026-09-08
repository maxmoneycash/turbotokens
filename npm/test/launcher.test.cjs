"use strict";

const assert = require("node:assert/strict");
const { once } = require("node:events");
const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");
const runtime = process.env.TURBOTOKENS_TEST_NODE || process.execPath;

function fixture(t, installBinary = true) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "turbotokens launcher "));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  fs.mkdirSync(path.join(directory, "bin"));
  fs.mkdirSync(path.join(directory, "vendor"));
  const launcher = path.join(directory, "bin", "turbotokens.js");
  fs.copyFileSync(path.join(__dirname, "../bin/turbotokens.js"), launcher);
  const binary = path.join(directory, "vendor", process.platform === "win32" ? "turbotokens.exe" : "turbotokens");
  // Use Node as a controllable native child without requiring a Rust build.
  if (installBinary) {
    try {
      fs.linkSync(runtime, binary);
    } catch {
      fs.copyFileSync(runtime, binary);
    }
  }
  return { directory, launcher, binary };
}

function run(launcher, args, options = {}) {
  return spawnSync(runtime, [launcher, ...args], {
    encoding: "utf8", timeout: 15000, ...options,
  });
}

test("forwards arguments and standard streams without a shell", (t) => {
  const { launcher } = fixture(t);
  const args = ["a b", "'quotes'", '"double quotes"', "$HOME", "$(echo unsafe)", "日本語"];
  const code = 'process.stdout.write(JSON.stringify(process.argv.slice(1))); process.stderr.write(require("fs").readFileSync(0));';
  const result = run(launcher, ["-e", code, "--", ...args], { input: "input through stdin\n" });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), args);
  assert.equal(result.stderr, "input through stdin\n");
});

test("preserves a nonzero native exit status", (t) => {
  const { launcher } = fixture(t);
  const result = run(launcher, ["-e", "process.exit(23)"]);
  assert.equal(result.status, 23);
  assert.equal(result.stderr, "");
});

test("reports a missing native binary", (t) => {
  const { launcher } = fixture(t, false);
  const result = run(launcher, ["--version"]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /binary is missing/);
  assert.match(result.stderr, /--ignore-scripts=false/);
});

test("reports a native binary that cannot be executed", { skip: process.platform === "win32" }, (t) => {
  const { launcher, binary } = fixture(t, false);
  fs.writeFileSync(binary, "not executable", { mode: 0o644 });
  const result = run(launcher, ["--version"]);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /EACCES/);
});

test("preserves a signal from the native child", { skip: process.platform === "win32" }, (t) => {
  const { launcher } = fixture(t);
  const result = run(launcher, ["-e", 'process.kill(process.pid, "SIGTERM")']);
  assert.equal(result.signal, "SIGTERM");
  assert.equal(result.status, null);
});

for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"]) {
  test(`forwards ${signal} sent only to the launcher and waits for cleanup`, {
    skip: process.platform === "win32", timeout: 30000,
  }, async (t) => {
    const { directory, launcher } = fixture(t);
    const cleanupFile = path.join(directory, "cleaned-up");
    const code = `
      const fs = require("fs");
      const [cleanupFile, signal] = process.argv.slice(1);
      process.on(signal, () => {
        setTimeout(() => {
          fs.writeFileSync(cleanupFile, signal);
          process.exit(0);
        }, 25);
      });
      process.stdout.write(String(process.pid));
      setInterval(() => {}, 1000);
    `;
    const wrapper = spawn(runtime, [launcher, "-e", code, cleanupFile, signal], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let childPid;
    t.after(() => {
      wrapper.kill("SIGKILL");
      if (childPid) {
        try { process.kill(childPid, "SIGKILL"); } catch (error) {
          if (error.code !== "ESRCH") throw error;
        }
      }
    });
    const exited = once(wrapper, "exit");
    const [ready] = await once(wrapper.stdout, "data");
    childPid = Number(ready.toString());
    assert(childPid > 0);
    wrapper.kill(signal);
    assert.deepEqual(await exited, [0, null]);
    assert.equal(fs.readFileSync(cleanupFile, "utf8"), signal);
    assert.throws(() => process.kill(childPid, 0), { code: "ESRCH" });
  });
}
