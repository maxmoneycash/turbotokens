#!/usr/bin/env node
"use strict";

const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");

const binary = path.join(__dirname, "..", "vendor", process.platform === "win32" ? "turbotokens.exe" : "turbotokens");
if (!fs.existsSync(binary)) {
  console.error("turbotokens: binary is missing. Reinstall with npm install -g turbotokens --ignore-scripts=false.");
  process.exit(1);
}
const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });
const handlers = new Map();
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  const handler = () => child.kill(signal);
  handlers.set(signal, handler);
  process.on(signal, handler);
}
child.on("error", (error) => {
  console.error(`turbotokens: ${error.message}`);
  process.exit(1);
});
child.on("exit", (status, signal) => {
  for (const [name, handler] of handlers) process.removeListener(name, handler);
  if (signal) process.kill(process.pid, signal);
  else process.exit(status === null ? 1 : status);
});
