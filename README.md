# JanusOS — Authorized Services Platform

Janus is being built into a production platform for **authorized** digital-evidence
services. A paid license enables product features; it never replaces a customer's
written authorization or engagement scope.

> **Status: pre-production.** Read the honesty rules below before relying on any
> output. Most of the Lua library is **uncertified demo content**, not working
> services. Only the read-only evidence family is on a certification path.

## Two binaries, two very different trust levels

| Binary | Purpose | Trust |
|--------|---------|-------|
| `janus-web` | Dashboard + module launcher on port 5000. | **Untrusted display.** Its Lua runner produces *simulated* output and is disabled unless `JANUS_DEMO_MODE=1`. |
| `janus-runtime` | Authenticated API on port 8080. Enforces license + engagement + certification before executing anything. | **The only path to real operations.** |

Never treat dashboard terminal output as evidence that an operation ran. Real
results come from the authenticated runtime API.

---

## Quick Start

```bash
git clone https://github.com/Wizardwalt/Janus-OS
cd Janus-OS
cargo run --bin janus-web
# Open http://localhost:5000
```

The dashboard serves `web/`, lists the module library, and connects a WebSocket at
`/ws`. It resolves the repository root automatically, so it also works when
started from another directory or as a release binary (`JANUS_ROOT` overrides).

**Requirements:** Rust stable 1.80+, pkg-config, OpenSSL, libudev

```bash
# Arch / Manjaro
pacman -S rust pkg-config openssl libudev

# Ubuntu / Debian
apt install rustup pkg-config libssl-dev libudev-dev build-essential
```

### Seeing simulated module output (optional, explicitly a demo)

```bash
JANUS_DEMO_MODE=1 cargo run --bin janus-web
```

Demo output is labelled in the UI and performs **no** device, file, or network
operation. It exists to exercise the launcher, not to demonstrate capability.

### Running the real, authorized runtime

```bash
# Public key only. The matching private signing key stays in the licensing
# service and must never be committed or deployed to a customer device.
export JANUS_LICENSE_PUBLIC_KEY="<base64 Ed25519 public key>"
cargo run --bin janus-runtime -- --bind 127.0.0.1 --port 8080
```

Then, through the dashboard's **JANUS RUNTIME ACCESS** panel (or the API directly):

1. `POST /auth/bootstrap` — create the first organization administrator (once).
2. `POST /licenses/import` — import a signed license for your organization.
3. Create a **Reviewer** user and `POST /modules/certifications` to approve an
   exact module ID + SHA-256 pair.
4. `POST /engagements` — record the customer's authorization reference, time
   window, approved assets, and approved evidence paths.
5. `POST /execute` — run a certified module against an approved target.

Every one of those checks is enforced by `janus_core::authorize_execution`. If any
fails, execution is refused and an `EXECUTION_DENIED` audit event is recorded.

The dashboard reaches the runtime through a same-origin proxy at
`/runtime-api/*`, so the browser never needs a `localhost` address. This is what
makes the dashboard work on Replit and behind HTTPS. Override the upstream with
`JANUS_RUNTIME_URL` (default `http://127.0.0.1:8080`).

---

## What actually works today

**Real and enforced**
- Argon2id password hashing, expiring sessions, organization-scoped roles.
- Ed25519 signed-license verification (public key only on the appliance).
- Engagement scope with exact-match approved assets and evidence paths.
- Module certification bound to a SHA-256 content hash; editing a module
  invalidates its certification.
- Organization-scoped audit trail, including denied execution attempts.
- **Read-only evidence collection** (`plugins/evidence_readonly/`, 5 modules):
  real SHA-256, `stat`, directory listing, bounded hex reads, and a
  chain-of-custody manifest — all confined to approved paths after symlink
  resolution. See `janus-runtime/src/evidence.rs`.

**Simulated or unavailable — never reported as working**
- The `janus-web` Lua runner (`janus.shell`, `janus.adb`, hardware stubs,
  `perform_*_action`, `unleash_*_power`) emits invented output. Demo mode only.
- `janus.shell`, `janus.adb`, and `janus.exec_system_cmd` in the **runtime**
  sandbox raise an error: no certified implementation exists.
- GPIO, cellular, RF, and sensor adapters report `unsupported`. Serial reports
  `available` only when the configured device path exists; there is no production
  serial driver yet.
- The bulk `modules/` and `plugins/` Lua library is **uncertified**. Most files
  only print strings. They are not services.

---

## Architecture

```
Janus-Omega-OS/
├── janus-web/               # Rust axum server — the main binary
│   └── src/main.rs          # WebSocket + static file serving + Lua VM
├── web/                     # Frontend (served at localhost:5000)
│   ├── index.html           # Boot screen → launcher
│   ├── app.js               # Android-style module launcher UI
│   ├── style.css            # Dark terminal aesthetic
│   ├── aria3d.js            # ARIA 3D avatar (Three.js human model)
│   ├── aria_profile.js      # Persistent profile + tattoo/marking system
│   └── aria_setup.js        # First-time setup wizard
├── plugins/                 # Lua plugin modules (per-category subdirs)
│   ├── forensics/
│   ├── sigint/
│   ├── network_warfare/
│   ├── mobile_offense/
│   ├── cyber_warfare/
│   ├── hardware_glitch/
│   ├── titan_exclusive/
│   └── osint_oracle/
├── modules/                 # Extended Lua module library (2,000+ scripts)
│   ├── god_tier/            # 1,000 supreme-tier modules
│   ├── legendary/
│   ├── forensics_recovery/
│   ├── sigint/
│   ├── network_warfare/
│   ├── mobile_offense/
│   └── tactical_defensive/
├── apocalypse_engineering/  # Advanced engineering module suite
├── core/                    # Core Lua infrastructure
├── janus-core/              # Shared Rust types
├── janus-runtime/           # Runtime crate
└── janus-tui/               # Terminal UI crate
```

---

## Features

### ARIA — AI Companion
- **Human-like 3D avatar** — full anatomy, skin tones, clothing, holographic aura (Three.js)
- **First-time setup wizard** — name, skin/hair/eye colour, outfit, first tattoo. Runs once, then locked in permanently
- **Permanent modification system** — markings are additive-only, stored in `localStorage`; laser removal module required to remove
- **Contextual intelligence** — responds to forensics queries, network questions, casual conversation
- **Idle thoughts** broadcast every 9 seconds

### Module Launcher
- **10,631 Lua files** on disk across 27 categories, parsed by a real **Lua 5.4 VM** (`mlua`).
  This is a file count, **not** a count of working services. The overwhelming
  majority are uncertified demo scripts.
- Live terminal output streamed line-by-line via **WebSocket**.
- Android-style app tiles → module list → live terminal output.
- The launcher states on-screen whether the server is in demo or production mode.

### Lua APIs — runtime sandbox (real)

`janus-runtime` exposes only this surface. It loads no `io`, `os`, `package`, or
`debug` library, so a module cannot reach the filesystem or a shell except
through the approved evidence scope.

| Function | Behaviour |
|----------|-----------|
| `print(...)` | Captured into the returned `logs` array |
| `janus.log(...)` | Captured into the returned `logs` array |
| `janus.evidence.hash(path)` | **Real** SHA-256 + byte count of an approved file |
| `janus.evidence.stat(path)` | **Real** filesystem metadata |
| `janus.evidence.list(path)` | **Real** directory listing; symlinks reported, not followed |
| `janus.evidence.read(path, offset, len)` | **Real** bounded read as hex (max 1 MiB/call) |
| `janus.shell` / `janus.adb` / `janus.exec_system_cmd` | **Raises an error.** No certified implementation. |
| Any unknown global | **Raises an error.** Never auto-stubbed to success. |

Every `janus.evidence.*` call requires an absolute path that resolves — after
symlink canonicalization — inside a directory the customer approved for the
active engagement. Paths outside that scope, relative paths, and `..` traversal
are refused.

### Lua APIs — dashboard demo runner (simulated)

`janus-web` contains a legacy runner whose `janus.shell`, `janus.adb`,
`read_rotary_dial`, `wait_for_haptic_confirmation`, `perform_*_action`, and
`unleash_*_power` functions return **invented** values. `wait_for_haptic_confirmation`
always returns `true`; it is **not** an authorization control. Unknown globals are
auto-stubbed. This runner is disabled unless `JANUS_DEMO_MODE=1` and must never be
used to justify a real operation.

### Hardware Fleet — target specifications, not supported devices

These are design targets. **No production driver ships today.** The runtime
reports GPIO, cellular, RF, and sensors as `unsupported`, and serial as
`available` only when the configured device path exists.

| Device | Spec | Driver status |
|--------|------|---------------|
| **Pandora Mk.1** | RP2040 USB glitcher / voltage injector | Not implemented |
| **Pandora Omega** | Radxa CM5 cyberdeck (8-core, 16 GB RAM) | Not implemented |
| **Pandora Titan** | 7" 21:9 forearm unit · 5G/LTE · Wi-Fi 6E · IP68 | Not implemented |

**Titan features** (Neural-Sync, AR-HUD, CBRN, Kinetic Harvester, Ghost-Net,
Faraday cage, post-quantum crypto, blackbox recorder) are **aspirational**. Any
current output claiming them is simulated.

---

## Module certification

Modules are certified one family at a time. A certification records an exact
module ID and SHA-256 content hash; changing the file invalidates it.

| Family | Status |
|--------|--------|
| `evidence_readonly` — hash, stat, list, read, manifest | Implemented and tested; awaiting reviewer sign-off |
| Everything else | `pending_review` — not executable in production |

Inventory without executing anything:

```bash
./scripts/validate-modules.sh
```

To produce the review sheet for the read-only family — exact module IDs, SHA-256
hashes, and the API calls that would record a decision:

```bash
python3 scripts/certify-evidence.py            # review sheet
python3 scripts/certify-evidence.py --curl     # + ready-to-run curl commands
```

This script **certifies nothing**. Read the code, then record `production_approved`
through `POST /modules/certifications` as a user holding the Reviewer role.

---

## Development

```bash
# Compile check the whole workspace
cargo check --workspace

# Run the Rust test suite (license, authorization, evidence, sandbox)
cargo test --workspace

# Run the dashboard
cargo run --bin janus-web

# Release build
cargo build --release --bin janus-web
./target/release/janus-web        # resolves the repo root automatically
```

`janus-web` binds `0.0.0.0:5000` (override with `JANUS_WEB_PORT`), serves `web/`,
opens a WebSocket at `/ws`, and proxies `/runtime-api/*` to the runtime. SQLite is
bundled; no external service is required.

### Non-Rust verification tools

`tools/audit/` contains checks that run on a real Lua 5.4 interpreter (via the
`wasmoon` WebAssembly build) and therefore work even where a Rust toolchain is
unavailable. They **complement, not replace, `cargo test`**.

```bash
cd tools/audit && npm install
npm run syntax          # compile-check every .lua file; executes nothing
npm run test:evidence   # behaviour + scope tests for the read-only family
npm run test:sandbox    # confirm io/os/package escapes are blocked
```

One command runs everything available and reports honestly what it could not run:

```bash
./scripts/verify.sh
```

It prints a `PASS` / `FAIL` / `SKIP` line per check and ends with an explicit
**NOT VERIFIED** list. Where no Rust toolchain exists, `cargo check` and
`cargo test` are reported as **skipped, never as passed**. A separate structural
check (`tools/audit/check_rust_structure.py`) verifies Rust delimiter balance; it
is not a substitute for compiling.

See [`docs/production/AUDIT-2026-11-05.md`](docs/production/AUDIT-2026-11-05.md)
for the full audit, what was tested, and what remains unfinished.

---

## ISO Build

JanusOS boots as an Arch Linux live ISO via `archiso`. The system auto-logs in as root, starts ADB, and launches the GUI in Alacritty with JetBrains Mono font. The GitHub Actions workflow in `.github/workflows/` handles CI validation on every push to `main`.

---

## License

Proprietary — Wizardwalt. All rights reserved.
