#!/usr/bin/env node
/*
 * Behavioural tests for the read-only evidence module family.
 *
 * Runs the real Lua 5.4 interpreter (wasmoon) against the real module sources,
 * with a `janus.evidence` host API that mirrors janus-runtime/src/evidence.rs:
 * absolute-path requirement, approved-root containment after symlink resolution,
 * real SHA-256, real stat/list/read, and no write or shell capability.
 *
 * This validates module logic and scope enforcement. It does NOT replace
 * `cargo test` for the Rust implementation.
 */
'use strict';
const { LuaFactory } = require('wasmoon');
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');

const REPO = path.resolve(__dirname, '..', '..');
const MODULES = path.join(REPO, 'plugins', 'evidence_readonly');

let pass = 0, fail = 0;
function check(name, condition, detail) {
  if (condition) { pass++; console.log(`  PASS  ${name}`); }
  else { fail++; console.log(`  FAIL  ${name}${detail ? ' :: ' + detail : ''}`); }
}

function makeTempRoot() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'janus-evidence-'));
}

// Host API mirroring evidence.rs semantics.
function buildApi(L, approvedRoots) {
  const logs = [];
  const resolve = (p) => {
    if (typeof p !== 'string' || p.trim() === '') throw new Error('evidence path must not be empty');
    if (!path.isAbsolute(p)) throw new Error(`evidence path must be absolute: ${p}`);
    let real;
    try { real = fs.realpathSync(p); } catch (e) { throw new Error(`evidence path is not readable: ${p}`); }
    const inside = approvedRoots.some((root) => {
      const r = fs.realpathSync(root);
      return real === r || real.startsWith(r + path.sep);
    });
    if (!inside) throw new Error(`evidence path is outside the approved engagement scope: ${p}`);
    return real;
  };

  L.global.set('__log', (s) => { logs.push(String(s)); });
  L.global.set('__hash', (p) => {
    const real = resolve(p);
    const data = fs.readFileSync(real);
    return { path: real, sha256: crypto.createHash('sha256').update(data).digest('hex'), size_bytes: data.length };
  });
  L.global.set('__stat', (p) => {
    const real = resolve(p);
    const st = fs.lstatSync(real);
    return { path: real, size_bytes: st.size, is_file: st.isFile(), is_dir: st.isDirectory(),
             readonly: !(st.mode & 0o200), modified_unix: Math.floor(st.mtimeMs / 1000) };
  });
  L.global.set('__list', (p) => {
    const real = resolve(p);
    return fs.readdirSync(real, { withFileTypes: true }).map((e) => ({
      name: e.name, is_file: e.isFile(), is_dir: e.isDirectory(), is_symlink: e.isSymbolicLink(),
    })).sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
  });
  L.global.set('__read', (p, offset, length) => {
    const real = resolve(p);
    const off = offset || 0, len = length || 4096;
    if (len <= 0) throw new Error('read length must be greater than zero');
    if (len > 1048576) throw new Error('read length exceeds the per-call limit');
    const fd = fs.openSync(real, 'r');
    try {
      const buf = Buffer.alloc(len);
      const n = fs.readSync(fd, buf, 0, len, off);
      return { path: real, offset: off, bytes_read: n, hex: buf.subarray(0, n).toString('hex') };
    } finally { fs.closeSync(fd); }
  });
  return logs;
}

const PRELUDE = `
-- Mirror new_sandbox_lua(): remove ambient filesystem/process access.
io = nil; os = nil; package = nil; require = nil; module = nil
dofile = nil; loadfile = nil; debug = nil
print = function(...)
  local parts = {}
  for i = 1, select('#', ...) do parts[#parts+1] = tostring((select(i, ...))) end
  __log(table.concat(parts, "\\t"))
end
janus = { log = function(...)
  local parts = {}
  for i = 1, select('#', ...) do parts[#parts+1] = tostring((select(i, ...))) end
  __log(table.concat(parts, " "))
end }
janus.evidence = { hash = __hash, stat = __stat, list = __list, read = __read }
janus.shell = function() error("janus.shell is not available: no certified implementation is installed for this operation") end
janus.adb = function() error("janus.adb is not available: no certified implementation is installed for this operation") end
`;

async function runModule(factory, file, args, approvedRoots) {
  const L = await factory.createEngine();
  const logs = buildApi(L, approvedRoots);
  await L.doString(PRELUDE);
  L.global.set('__args', args || {});
  await L.doString('args = __args');
  const src = fs.readFileSync(file, 'utf8');
  L.global.set('__src', src);
  L.global.set('__name', path.basename(file));
  let outcome;
  try {
    const r = await L.doString(`
      local fn, err = load(__src, "@"..__name)
      if not fn then return { ok = false, error = "COMPILE: "..err } end
      local ok, res = pcall(fn)
      if not ok then return { ok = false, error = tostring(res) } end
      if type(res) == "table" then return { ok = true, result = res } end
      return { ok = true, result = {} }
    `);
    outcome = r;
  } catch (e) {
    outcome = { ok: false, error: 'ENGINE: ' + e.message };
  }
  const out = { ...outcome, logs: [...logs] };
  try { L.global.close(); } catch (_) {}
  return out;
}

(async () => {
  const factory = new LuaFactory();
  const root = makeTempRoot();
  const outside = makeTempRoot();
  const evidenceFile = path.join(root, 'evidence.bin');
  fs.writeFileSync(evidenceFile, Buffer.from('abc'));
  fs.writeFileSync(path.join(root, 'b.txt'), 'b');
  fs.writeFileSync(path.join(outside, 'secret.bin'), 'secret');
  const expectedSha = crypto.createHash('sha256').update('abc').digest('hex');

  console.log('\n== evidence.hash ==');
  let r = await runModule(factory, path.join(MODULES, 'evidence_hash.lua'), { path: evidenceFile }, [root]);
  check('hashes a real approved file', r.ok && r.result && r.result.sha256 === expectedSha, JSON.stringify(r).slice(0, 200));
  check('reports correct byte count', r.ok && r.result && r.result.size_bytes === 3);
  check('emits real log lines', r.logs.length >= 2, JSON.stringify(r.logs));
  check('marks result read_only', r.ok && r.result && r.result.read_only === true);

  r = await runModule(factory, path.join(MODULES, 'evidence_hash.lua'), { path: path.join(outside, 'secret.bin') }, [root]);
  check('refuses a file outside the approved scope', !r.ok && /outside the approved/.test(r.error || ''), r.error);

  r = await runModule(factory, path.join(MODULES, 'evidence_hash.lua'), { path: 'relative/path' }, [root]);
  check('refuses a relative path', !r.ok && /must be absolute/.test(r.error || ''), r.error);

  r = await runModule(factory, path.join(MODULES, 'evidence_hash.lua'), {}, [root]);
  check('refuses a missing path argument', !r.ok && /args\.path/.test(r.error || ''), r.error);

  r = await runModule(factory, path.join(MODULES, 'evidence_hash.lua'), { path: evidenceFile }, []);
  check('refuses when engagement approved no paths', !r.ok, r.error);

  r = await runModule(factory, path.join(MODULES, 'evidence_hash.lua'), { path: path.join(root, '..', path.basename(outside), 'secret.bin') }, [root]);
  check('refuses parent-directory traversal', !r.ok, r.error);

  console.log('\n== evidence.stat ==');
  r = await runModule(factory, path.join(MODULES, 'evidence_stat.lua'), { path: evidenceFile }, [root]);
  check('stats a real approved file', r.ok && r.result && r.result.size_bytes === 3 && r.result.is_file === true, JSON.stringify(r).slice(0, 200));
  r = await runModule(factory, path.join(MODULES, 'evidence_stat.lua'), { path: path.join(outside, 'secret.bin') }, [root]);
  check('refuses stat outside scope', !r.ok, r.error);

  console.log('\n== evidence.list ==');
  r = await runModule(factory, path.join(MODULES, 'evidence_list.lua'), { path: root }, [root]);
  check('lists a real approved directory', r.ok && r.result && r.result.entry_count === 2, JSON.stringify(r.result || r.error).slice(0, 200));
  r = await runModule(factory, path.join(MODULES, 'evidence_list.lua'), { path: outside }, [root]);
  check('refuses listing outside scope', !r.ok, r.error);

  console.log('\n== read-only guarantee ==');
  const before = crypto.createHash('sha256').update(fs.readFileSync(evidenceFile)).digest('hex');
  await runModule(factory, path.join(MODULES, 'evidence_hash.lua'), { path: evidenceFile }, [root]);
  await runModule(factory, path.join(MODULES, 'evidence_stat.lua'), { path: evidenceFile }, [root]);
  await runModule(factory, path.join(MODULES, 'evidence_list.lua'), { path: root }, [root]);
  const after = crypto.createHash('sha256').update(fs.readFileSync(evidenceFile)).digest('hex');
  check('evidence file is unchanged after all operations', before === after);
  check('no new files were created in the evidence root', fs.readdirSync(root).length === 2);

  console.log('\n== unavailable operations are honest ==');
  const L = await factory.createEngine();
  buildApi(L, [root]);
  await L.doString(PRELUDE);
  const shellResult = await L.doString('local ok, err = pcall(function() return janus.shell("id") end); if ok then return "ALLOWED" else return tostring(err) end');
  check('janus.shell raises instead of simulating success', /not available/.test(shellResult), shellResult);
  const unknown = await L.doString('local ok = pcall(function() return totally_unknown_fn() end); if ok then return "ALLOWED" else return "REFUSED" end');
  check('unknown globals are NOT auto-stubbed to success', unknown === 'REFUSED', unknown);
  try { L.global.close(); } catch (_) {}

  fs.rmSync(root, { recursive: true, force: true });
  fs.rmSync(outside, { recursive: true, force: true });

  console.log(`\n${pass} passed, ${fail} failed`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error('HARNESS FATAL', e); process.exit(1); });
