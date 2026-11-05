#!/usr/bin/env node
/*
 * Compile-only Lua 5.4 syntax audit.
 *
 * Uses load(), which compiles a chunk WITHOUT executing it, so no module code
 * ever runs during this check.
 *
 * The Lua engine is a WebAssembly build. Creating and discarding thousands of
 * states in one process degrades and can stall, so the corpus is split into
 * chunks handled by short-lived worker processes. A stalled or crashed chunk is
 * reported as "unverified" rather than silently counted as a pass.
 */
'use strict';
const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

const ROOT = process.argv[2] || path.resolve(__dirname, '..', '..');
const OUT = process.argv[3] || path.join(ROOT, 'reports', 'lua-syntax.json');
const CHUNK = Number(process.env.JANUS_CHUNK || 400);
const CHUNK_TIMEOUT_MS = Number(process.env.JANUS_CHUNK_TIMEOUT_MS || 120000);

function walk(dir, acc = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (['.git', 'target', 'node_modules', 'reports'].includes(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, acc);
    else if (entry.name.endsWith('.lua')) acc.push(full);
  }
  return acc;
}

// ── Worker mode: compile one chunk and print JSON ────────────────────────────
if (process.argv[2] === '--worker') {
  const listFile = process.argv[3];
  const root = process.argv[4];
  const files = fs.readFileSync(listFile, 'utf8').split('\n').filter(Boolean);
  (async () => {
    const { LuaFactory } = require('wasmoon');
    const factory = new LuaFactory();
    const failures = [];
    let ok = 0;
    for (const rel of files) {
      const src = fs.readFileSync(path.join(root, rel), 'utf8');
      // A fresh engine per file is required. Reusing one engine across many
      // chunks corrupts the WebAssembly heap and aborts with "memory access out
      // of bounds", which would otherwise be misreported as a module defect.
      const engine = await factory.createEngine();
      let result;
      try {
        engine.global.set('__src', src);
        engine.global.set('__name', rel);
        result = await engine.doString(
          'local fn, err = load(__src, "@"..__name); if fn then return "OK" else return err end'
        );
      } catch (error) {
        result = 'ENGINE: ' + error.message;
      }
      try { engine.global.close(); } catch (_) {}
      if (result === 'OK') ok++;
      else failures.push({ file: rel, error: String(result).slice(0, 300) });
    }
    process.stdout.write(JSON.stringify({ ok, failures }));
  })().catch((error) => {
    process.stdout.write(JSON.stringify({ ok: 0, failures: [], fatal: String(error.message || error) }));
  });
  return;
}

// ── Driver mode ──────────────────────────────────────────────────────────────
const files = walk(ROOT).sort().map((f) => path.relative(ROOT, f));
const os = require('os');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'janus-syntax-'));
let ok = 0;
const failures = [];
const unverified = [];

for (let start = 0; start < files.length; start += CHUNK) {
  const slice = files.slice(start, start + CHUNK);
  const listFile = path.join(tmp, `chunk-${start}.txt`);
  fs.writeFileSync(listFile, slice.join('\n'));
  const child = spawnSync(
    process.execPath,
    [__filename, '--worker', listFile, ROOT],
    { timeout: CHUNK_TIMEOUT_MS, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }
  );
  let parsed = null;
  try { parsed = JSON.parse(child.stdout); } catch (_) {}
  if (parsed && !parsed.fatal) {
    ok += parsed.ok;
    failures.push(...parsed.failures);
  } else {
    // Never count an unrunnable chunk as a pass.
    unverified.push(...slice);
  }
  process.stderr.write(`\r  ${Math.min(start + CHUNK, files.length)}/${files.length} checked, ${failures.length} failed, ${unverified.length} unverified   `);
}
process.stderr.write('\n');
fs.rmSync(tmp, { recursive: true, force: true });

const report = {
  total: files.length,
  ok,
  failed: failures.length,
  unverified: unverified.length,
  failures,
  unverified_files: unverified,
};
fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.writeFileSync(OUT, JSON.stringify(report, null, 2));
console.log(`TOTAL=${files.length} OK=${ok} FAILED=${failures.length} UNVERIFIED=${unverified.length}`);
for (const f of failures.slice(0, 20)) console.log(`  ${f.file} :: ${f.error}`);
console.log(`report: ${OUT}`);
process.exit(failures.length > 0 || unverified.length > 0 ? 1 : 0);
