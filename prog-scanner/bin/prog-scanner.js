#!/usr/bin/env node
"use strict";
// Launcher for prog-scanner. It downloads the prebuilt, read-only Windows
// program from this project's GitHub Release, verifies its SHA-256, runs it,
// and then deletes the temporary copy. It installs nothing.

const https = require("https");
const fs = require("fs");
const os = require("os");
const path = require("path");
const crypto = require("crypto");
const { spawnSync } = require("child_process");

const pkg = require("../package.json");

function repoSlug() {
  const url = (pkg.repository && (pkg.repository.url || pkg.repository)) || "";
  const m = /github\.com[/:]([^/]+\/[^/.]+)/.exec(url);
  if (!m || m[1].startsWith("YOUR_USERNAME")) {
    fail("package.json 'repository' still has the placeholder YOUR_USERNAME. Edit it first.");
  }
  return m[1];
}

function fail(msg) {
  console.error("prog-scanner: " + msg);
  process.exit(1);
}

function get(url, redirects = 5) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "prog-scanner-launcher" } }, (res) => {
        if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location) {
          res.resume();
          if (redirects <= 0) return reject(new Error("too many redirects"));
          return resolve(get(new URL(res.headers.location, url).toString(), redirects - 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error("HTTP " + res.statusCode + " for " + url));
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

(async () => {
  if (process.platform !== "win32") fail("this tool only works on Windows.");

  const base = `https://github.com/${repoSlug()}/releases/download/v${pkg.version}`;
  const asset = "prog-scanner-x86_64-windows.exe";

  let exe, sumText;
  try {
    console.error("Downloading prog-scanner v" + pkg.version + " ...");
    [exe, sumText] = await Promise.all([get(`${base}/${asset}`), get(`${base}/${asset}.sha256`)]);
  } catch (e) {
    fail("download failed: " + e.message);
  }

  const expected = sumText.toString("utf8").trim().split(/\s+/)[0].toLowerCase();
  const actual = crypto.createHash("sha256").update(exe).digest("hex");
  if (expected !== actual) fail("checksum mismatch - refusing to run the file.");

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "prog-scanner-"));
  const file = path.join(dir, "prog-scanner.exe");
  fs.writeFileSync(file, exe);

  let code = 1;
  try {
    const r = spawnSync(file, process.argv.slice(2), { stdio: "inherit" });
    code = r.status === null ? 1 : r.status;
  } finally {
    try { fs.rmSync(dir, { recursive: true, force: true }); } catch (_) {}
  }
  process.exit(code);
})();
