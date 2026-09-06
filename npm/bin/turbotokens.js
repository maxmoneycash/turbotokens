#!/usr/bin/env node
"use strict";

const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const binary = path.join(__dirname, "..", "vendor", process.platform === "win32" ? "turbotokens.exe" : "turbotokens");
if (!fs.existsSync(binary)) {
  console.error("turbotokens: binary is missing. Reinstall with npm install -g turbotokens --ignore-scripts=false.");
  process.exit(1);
}
const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`turbotokens: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) process.kill(process.pid, result.signal);
else process.exit(result.status === null ? 1 : result.status);
