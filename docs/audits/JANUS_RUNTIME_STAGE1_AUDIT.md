# Janus OS — Stage 1 Runtime Reality Audit

**Status:** Source-level audit complete for the inspected default-branch runtime path; device/ISO integration verification remains open.  
**Scope:** `janus-runtime`, `janus-web`, production Lua sandbox, plugin loading/execution, API boundary, and hardware adapters.

## Executive summary

The current Janus-OS default branch has a meaningful protected-runtime foundation. It is not accurate to describe the entire system as a simulation: a separate Rust runtime daemon, authenticated API handlers, SQLite state, a restricted Lua plugin sandbox, and a real serial-port adapter exist in source.

It is equally important not to overstate readiness. The legacy WebSocket module runner is explicitly demo-only. Several advertised hardware adapters are explicit stubs. The bootable ISO claim, real device integration, full frontend-to-runtime behavior, and recovery under process/device failure have not been proven by the automated checks inspected here.

## Capability matrix

| Capability | Evidence in current source | Stage 1 classification | Remaining proof/work |
|---|---|---|---|
| Runtime daemon | `janus-runtime/src/main.rs`; separate `janus-runtime` binary | Implemented; CI-built | Exercise startup, shutdown, restart, bad config, and DB failure in integration tests |
| Web dashboard | `janus-web` crate and `web/` assets | Implemented; CI-built | Verify all production UI paths against live runtime, not demo WebSocket |
| Runtime launcher | `scripts/run-janus.sh` builds both binaries, waits for `/health`, cleans up child processes | Implemented; shell syntax checked | Test early crash, timeout, signals, port collision, and restart behavior |
| Production plugin discovery | `janus-runtime/src/plugin.rs` loads Lua plugins and metadata | Implemented; workspace tests pass | Test malformed manifests, symlink/path boundaries, oversized plugins, duplicate IDs, and reload behavior |
| Production Lua sandbox | `janus-runtime/src/lua.rs` creates a fresh VM, removes ambient loaders/process/debug globals, sets instruction budget, exposes only `args` and `janus.log` | Implemented; source-level controls and unit tests exist | Add memory/output limits and adversarial tests for huge tables, deeply nested data, and instruction-budget edge cases |
| Capability authorization | `janus-runtime/src/executor.rs` denies every requested capability except `logging`; manifest declarations are not grants | Implemented; unit tests cover denied capabilities | Add end-to-end tests proving unauthorized execution is rejected and audit failure fails closed |
| Core Lua startup modules | `LuaEnv::load_core_modules` executes files from the configured core directory in a separate Lua environment; a bad file is logged and startup continues | Implemented but trusted-code assumption is not explicit in config | Define this directory as trusted/admin-managed only, or sandbox it; make required core-module failures fail startup |
| Legacy module browser | README states legacy WebSocket runner is available only with `JANUS_DEMO_MODE=1` | Demo/simulated | Keep unmistakably labelled; never use its output as evidence of real device work |
| Serial adapter | `hardware.rs` opens configured serial port at 115200 baud with 500 ms IO timeout and reports actual connection state | Implemented but hardware-unverified | Test with supported real hardware and record model/driver/OS |
| GPIO | Methods explicitly return “not implemented”; status is unsupported | Stub/unsupported | Do not show ready or claim pin operations |
| Cellular | Scan explicitly returns “not implemented”; status is unsupported | Stub/unsupported | Add a real supported adapter only with test hardware |
| RF | Scan explicitly returns “not implemented”; status is unsupported | Stub/unsupported | Add a real supported adapter only with test hardware |
| Sensors / GPS | Read methods explicitly return “not implemented”; status is unsupported | Stub/unsupported | Add a real supported adapter only with test hardware |
| ADB / Android servicing | No production ADB host capability is exposed in the inspected plugin sandbox; legacy demo ADB is simulated | Demo/unsupported in production | Design an explicit, authenticated device adapter with per-device consent and safe command allowlist |
| ArchISO / boot | Janus-Omega-OS README describes an ArchISO build; no successful current boot test is evidenced by Janus-OS CI | Unverified | Locate and build the actual profile; boot ISO in a VM; record artifact hash and test checklist |
| Recovery / backups | Janus-OS runtime has backup/restore code in main; separate PR history includes encrypted backup and recovery work | Implemented in code; integration unverified here | Run round-trip, tamper, wrong-key, interrupted restore, and database compatibility tests |

## Verified CI evidence

The Stage 1 CI change in PR #12 was run successfully. The completed workflow reported success for:

- `cargo check --bin janus-web`
- `cargo check --workspace`
- `cargo test --workspace`
- required frontend asset presence
- browser JavaScript syntax checks
- launcher shell syntax
- module count check
- release build of both `janus-runtime` and `janus-web`
- existence of both release binaries

This proves the checked-out source compiled and its current automated tests passed in GitHub's Ubuntu runner. It does **not** prove the ISO boots, a physical device is supported, or every UI/API path behaves correctly on hardware.

## Confirmed risks and corrective actions

### 1. Core Lua modules have a different trust boundary

The production plugin sandbox is instruction-limited and narrowly exposed. The startup `LuaEnv::load_core_modules` path is separate and currently executes configured core Lua files without the same instruction budget or capability restrictions. It logs a failed module and continues.

Action:
- Treat the core-module directory as privileged, trusted, administrator-managed code.
- Document ownership and write permissions for that directory.
- Add a startup test for malformed and hanging core modules.
- Decide whether core-module failures are fatal or explicitly optional; do not silently imply all core modules loaded.

### 2. Sandbox resource limits need completion

An instruction budget and table-depth bound exist, but the inspected code does not show a configured Lua memory quota or an explicit maximum serialized result size.

Action:
- Add memory and result-size limits.
- Add tests for oversized output, deeply nested tables, large strings/tables, and infinite loops.
- Keep capability authorization separate from classification and plugin metadata.

### 3. Hardware readiness must remain evidence-based

Only configured serial hardware can currently become available. GPIO, cellular, RF, and sensors explicitly report unsupported and return errors rather than fabricated readings.

Action:
- Keep those adapters marked unsupported until real drivers and test evidence exist.
- Do not promote demo ADB/shell or simulated hardware responses into production.
- Require model/driver-specific test records for each added adapter.

### 4. ISO and device servicing remain release blockers

A successful Rust workspace CI run is not an ISO build or boot test.

Action:
- Identify the authoritative ArchISO profile and dependency source.
- Reproduce the ISO build from a clean environment.
- Boot it in a VM and verify persistence/volatile behavior as designed.
- Test only supported, owned devices for ADB and servicing workflows.

## Stage 1 exit criteria

The runtime audit can be closed when all of the following have evidence:

1. The CI checks above remain green on the final candidate commit.
2. Startup and shutdown/restart integration tests cover runtime and web services.
3. Core Lua trust policy and failure behavior are explicit and tested.
4. Lua sandbox memory, instruction, and output limits have adversarial tests.
5. API authentication, role/engagement/asset scope, and audit persistence are tested end-to-end.
6. Unsupported hardware remains clearly unsupported; supported serial adapter has a real-device test record.
7. Demo-only UI and outputs cannot be mistaken for production results.
8. ArchISO builds reproducibly and boots in a VM; hardware-specific claims remain separately qualified.

## Limits of this audit

This report is based on source inspection and the cited CI workflow result. No physical device was connected, no ISO was booted, no penetration test was performed, and no production deployment was attempted. Those items remain explicitly unverified.
