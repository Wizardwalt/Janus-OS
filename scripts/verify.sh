#!/usr/bin/env bash
# Janus verification — one command, honest results.
#
# Runs every check that is available in this environment and clearly separates
# what was VERIFIED from what was SKIPPED. Never reports a skipped check as pass.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
REPO="$PWD"
PASS=0; FAIL=0; SKIP=0
declare -a SKIPPED=()

hr() { printf '%s\n' "------------------------------------------------------------"; }
ok()   { printf '  [PASS] %s\n' "$1"; PASS=$((PASS+1)); }
bad()  { printf '  [FAIL] %s\n' "$1"; FAIL=$((FAIL+1)); }
skip() { printf '  [SKIP] %s\n' "$1"; SKIP=$((SKIP+1)); SKIPPED+=("$1"); }

hr; echo "JANUS VERIFICATION"; echo "repo: $REPO"; hr

# ── 1. Rust toolchain ────────────────────────────────────────────────────────
echo; echo "1. Rust build and tests"
if command -v cargo >/dev/null 2>&1; then
  echo "  cargo: $(cargo --version 2>&1)"
  echo "  -> cargo check --workspace"
  if cargo check --workspace --all-targets 2>&1 | tail -30; then
    ok "cargo check --workspace"
  else
    bad "cargo check --workspace"
  fi
  echo "  -> cargo test --workspace"
  if cargo test --workspace 2>&1 | tail -60; then
    ok "cargo test --workspace"
  else
    bad "cargo test --workspace"
  fi
else
  skip "cargo check --workspace (no Rust toolchain on PATH)"
  skip "cargo test --workspace (no Rust toolchain on PATH)"
  echo "     Install Rust, then re-run this script to execute the Rust suite."
fi

# ── 2. Lua verification (real Lua 5.4 via wasmoon) ───────────────────────────
echo; echo "2. Lua module verification (real Lua 5.4 interpreter)"
if command -v node >/dev/null 2>&1; then
  if [ ! -d tools/audit/node_modules ]; then
    echo "  installing wasmoon (local, dev-only)..."
    (cd tools/audit && npm install --no-audit --no-fund >/dev/null 2>&1)
  fi
  if [ -d tools/audit/node_modules/wasmoon ]; then
    if (cd tools/audit && node test_evidence_modules.js); then
      ok "read-only evidence module behaviour + scope enforcement"
    else
      bad "read-only evidence module behaviour"
    fi
    if (cd tools/audit && node test_sandbox_escape.js); then
      ok "sandbox blocks io/os/package escapes"
    else
      bad "sandbox escape hardening"
    fi
    echo "  -> compile-checking changed/new Lua files (executes nothing)"
    if (cd tools/audit && node check_lua_syntax.js "$REPO" "$REPO/reports/lua-syntax.json" 2>/dev/null | tail -12); then
      ok "Lua corpus compile check (see reports/lua-syntax.json)"
    else
      bad "Lua corpus compile check"
    fi
  else
    skip "Lua tests (wasmoon unavailable; npm install may be blocked)"
  fi
else
  skip "Lua tests (no Node.js on PATH)"
fi

# ── 3. Module inventory (never executes module code) ─────────────────────────
echo; echo "3. Module inventory"
if command -v python3 >/dev/null 2>&1; then
  if python3 scripts/inventory_modules.py --output reports/module-inventory.json; then
    ok "module inventory generated"
  else
    bad "module inventory"
  fi
else
  skip "module inventory (no python3)"
fi

# ── 4. Frontend syntax ───────────────────────────────────────────────────────
echo; echo "4. Dashboard JavaScript syntax"
if command -v node >/dev/null 2>&1; then
  jsfail=0
  for f in web/app.js web/runtime_api.js web/runtime_auth.js web/runtime_execution.js \
           web/runtime_status.js web/runtime_hardware.js web/runtime_admin.js \
           web/aria_profile.js web/aria3d.js web/aria_setup.js; do
    node --check "$f" >/dev/null 2>&1 || { echo "    syntax error: $f"; jsfail=1; }
  done
  if [ "$jsfail" -eq 0 ]; then ok "all dashboard JS parses"; else bad "dashboard JS syntax"; fi
else
  skip "dashboard JS syntax (no Node.js)"
fi

# ── 5. Secrets hygiene ───────────────────────────────────────────────────────
echo; echo "5. Signing-key hygiene"
if command -v git >/dev/null 2>&1; then
  if git ls-files | grep -qiE '\.(pem|key|p12|pfx|jks|keystore)$'; then
    bad "a private-key-looking file is tracked in Git"
    git ls-files | grep -iE '\.(pem|key|p12|pfx|jks|keystore)$'
  else
    ok "no private key files tracked in Git"
  fi
else
  skip "signing-key hygiene (no git)"
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo; hr
printf 'RESULT: %d passed, %d failed, %d skipped\n' "$PASS" "$FAIL" "$SKIP"
if [ "$SKIP" -gt 0 ]; then
  echo; echo "NOT VERIFIED in this environment:"
  for s in "${SKIPPED[@]}"; do printf '  - %s\n' "$s"; done
fi
hr
if [ "$FAIL" -gt 0 ]; then exit 1; fi
exit 0
