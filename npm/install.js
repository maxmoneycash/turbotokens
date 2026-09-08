#!/usr/bin/env node
// Download the matching release asset and verify it before extraction.
"use strict";

const { execFileSync } = require("child_process");
const { createHash } = require("crypto");
const fs = require("fs");
const https = require("https");
const os = require("os");
const path = require("path");
const { pipeline } = require("stream");
const { promisify } = require("util");
const pkg = require("./package.json");
const checksums = require("./checksums.json");
const REPO = "maxmoneycash/turbotokens";

const platforms = { darwin: "macos", linux: "linux", win32: "windows" };
const platform = platforms[process.platform];
const arch = process.arch;
const extension = process.platform === "win32" ? "zip" : "tar.gz";
const asset = `turbotokens-${platform}-${arch}.${extension}`;

function download(url, destination, redirects = 5) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
        response.resume();
        if (!redirects) return reject(new Error("too many download redirects"));
        const next = new URL(response.headers.location, url);
        if (next.protocol !== "https:") return reject(new Error("download redirect must use HTTPS"));
        download(next, destination, redirects - 1).then(resolve, reject);
        return;
      }
      if (response.statusCode !== 200) {
        response.resume();
        return reject(new Error(`download failed: HTTP ${response.statusCode}`));
      }
      promisify(pipeline)(response, fs.createWriteStream(destination)).then(resolve, reject);
    });
    request.setTimeout(30000, () => request.destroy(new Error("download timed out")));
    request.on("error", reject);
  });
}

async function install() {
  if (!platform || !["arm64", "x64"].includes(arch) || !checksums[asset]) {
    throw new Error(`no prebuilt binary for ${process.platform}-${arch}; build from source: https://github.com/${REPO}`);
  }
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "turbotokens-"));
  try {
    const archive = path.join(temporary, asset);
    const url = `https://github.com/${REPO}/releases/download/v${pkg.version}/${asset}`;
    console.log(`turbotokens: downloading v${pkg.version} for ${platform}-${arch}`);
    await download(url, archive);
    const digest = createHash("sha256").update(fs.readFileSync(archive)).digest("hex");
    if (digest !== checksums[asset]) throw new Error(`checksum mismatch for ${asset}`);
    const binary = process.platform === "win32" ? "turbotokens.exe" : "turbotokens";
    const destination = path.join(__dirname, "vendor");
    fs.mkdirSync(destination, { recursive: true });
    // Git Bash may put GNU tar first on PATH; it treats C: as a remote host
    // and does not handle the Windows ZIP asset. Use Windows' native bsdtar.
    const extractor = process.platform === "win32"
      ? path.join(process.env.SystemRoot || process.env.WINDIR || "C:\\Windows", "System32", "tar.exe")
      : "tar";
    execFileSync(extractor, [extension === "zip" ? "-xf" : "-xzf", archive, "-C", temporary, binary], { stdio: "inherit" });
    if (process.platform !== "win32") fs.chmodSync(path.join(temporary, binary), 0o755);
    fs.copyFileSync(path.join(temporary, binary), path.join(destination, binary));
    console.log(`turbotokens: installed v${pkg.version}`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

install().catch((error) => {
  console.error(`turbotokens: ${error.message}`);
  process.exitCode = 1;
});
