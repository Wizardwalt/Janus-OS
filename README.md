# JanusOS — Pandora Titan Field Intelligence Platform

A bootable Arch Linux forensics and intelligence platform. Rust web server, Android-style GUI, 2,600+ Lua modules, and ARIA — a persistent 3D AI companion with a permanent modification system.

---

## Quick Start

```bash
git clone https://github.com/Wizardwalt/Janus-OS.git
cd Janus-OS
bash scripts/run-janus.sh
# Open http://localhost:5000
```

The launcher builds and starts both services:
- **Janus Runtime API:** `127.0.0.1:8080`
- **Janus Web UI:** `0.0.0.0:5000`

The browser reaches the protected runtime through the web server's same-origin `/api` proxy. You do not need to expose port 8080 publicly.

**Requirements:** Rust stable 1.80+, pkg-config, OpenSSL, libudev

```bash
# Arch / Manjaro
pacman -S rust pkg-config openssl libudev

# Ubuntu / Debian
apt install rustup pkg-config libssl-dev libudev-dev build-essential
```

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
- **2,632 modules** across 17 categories executed by a real **Lua 5.4 VM** (`mlua`)
- Live terminal output streamed line-by-line via **WebSocket** — every `print()`, `overseer_speak()`, `janus.log()` surfaces in real time
- Android-style app tiles → module list → live terminal output

### Lua Sandbox APIs

The production runtime creates a fresh Lua VM for each plugin, applies an instruction limit, and exposes JSON input as `args`. The only host capability currently available is:

| Function | Behaviour |
|----------|-----------|
| `janus.log(msg)` | Writes a sanitized, length-limited message to the runtime log |
| `args` | Read-only-by-convention JSON input supplied to the plugin |

Plugins may declare capabilities in their manifest, but declarations are requests, not grants. Until a vetted implementation and administrator approval flow exist, production execution rejects every requested capability except `logging`. Shell, ADB, filesystem, network, and hardware access are not exposed by the production plugin sandbox.

The legacy WebSocket Lua runner is available only when `JANUS_DEMO_MODE=1` is explicitly set. Its shell/ADB and hardware responses are simulations for UI development, not real device operations. Do not use demo output as evidence that a scan, extraction, or hardware action occurred.

### ARIA status

ARIA's current chat replies and periodic thoughts are scripted interface text. They do not represent a connected language model, live network monitoring, radio scanning, or persistent learning. Live AI integration and verified telemetry are future integrations; the interface must not present simulated text as operational results.

### Hardware status

The production hardware API reports an adapter as ready only after a real driver is implemented and can verify its state. Serial, GPIO, cellular, RF, and sensor operations currently return explicit unsupported errors rather than simulated success.
---

## Development

```bash
# Run (hot-reload on file changes with cargo-watch)
cargo watch -x 'run --bin janus-web'

# Compile check
cargo check --bin janus-web

# Release build
cargo build --release --bin janus-web
./target/release/janus-web
```

The server binds `0.0.0.0:5000`, serves static files from `web/`, and opens a WebSocket at `/ws`. No external services required — SQLite is bundled.

---

## ISO Build

JanusOS boots as an Arch Linux live ISO via `archiso`. The system auto-logs in as root, starts ADB, and launches the GUI in Alacritty with JetBrains Mono font. The GitHub Actions workflow in `.github/workflows/` handles CI validation on every push to `main`.

---

## License

Proprietary — Wizardwalt. All rights reserved.
