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

| Function | Behaviour |
|----------|-----------|
| `print(...)` | Streamed to terminal |
| `overseer_speak(msg)` | `[OVERSEER] msg` |
| `janus.log(msg)` | `[JANUS] msg` |
| `janus.shell(cmd)` | Simulated shell output |
| `janus.adb(cmd)` | Simulated ADB output |
| `log_to_blackbox(tbl)` | Flight recorder entry |
| `read_rotary_dial()` | Hardware dial value (1–100) |
| `wait_for_haptic_confirmation(n)` | Returns `true` (authorised) |
| `perform_core_action(...)` | Core op → `{status="success"}` |
| `perform_advanced_action(...)` | Advanced op → `{status="success"}` |
| `perform_final_action(...)` | Terminal op → `{status="success"}` |
| `unleash_god_tier_power(...)` | God-tier power stub |
| `unleash_legendary_power(...)` | Legendary power stub |
| Any unknown global | Auto-stubbed via `__index` on `_G` — no crashes |

### Hardware Fleet

| Device | Spec |
|--------|------|
| **Pandora Mk.1** | RP2040 USB glitcher / voltage injector |
| **Pandora Omega** | Radxa CM5 cyberdeck (8-core, 16 GB RAM) |
| **Pandora Titan** | 7" 21:9 forearm Pip-Boy · 5G/LTE · Wi-Fi 6E · IP68 · MIL-STD-810H |

**Titan Exclusives:** Neural-Sync · AR-HUD · CBRN Suite · Kinetic Harvester · Ghost-Net mesh · Faraday cage · Quantum-resistant crypto · 24/7 blackbox recorder

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
