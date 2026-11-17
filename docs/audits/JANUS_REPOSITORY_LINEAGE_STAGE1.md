# Janus Repository Lineage — Stage 1 Evidence Report

**Status:** Evidence-backed provisional ownership recommendation  
**Date:** 2026-11-16  
**Repositories:** `Wizardwalt/Janus-OS` and `Wizardwalt/Janus-Omega-OS`

## Executive decision

Treat these as **two distinct GitHub repositories in one Janus product lineage**, not as interchangeable names for one repository.

For current protected-runtime engineering and Stage 1 release-readiness, use **`Wizardwalt/Janus-OS` as the working canonical implementation**. Preserve **`Wizardwalt/Janus-Omega-OS` as the original Janus-Omega lineage and a separately maintained repository** until its ArchISO, hardware, and project-history contents have been inventoried and a deliberate migration decision is made.

This is a provisional repository-ownership recommendation based on the checked default-branch files. It is **not proof of shared Git commit ancestry**, and it does not authorize a merge, rename, deletion, bulk copy, or retirement of either repository.

## Evidence observed

### Shared product architecture and likely code lineage

Both default branches expose the same Rust workspace member names:

- `janus-core`
- `janus-runtime`
- `janus-tui`
- `janus-web`

Both root `Cargo.toml` files declare the same workspace members, resolver, Rust edition, version, author, and most of the same workspace dependencies. Both READMEs describe the same Pandora Titan Field Intelligence Platform, ARIA companion, web interface, plugin tree, Lua module tree, and Rust crate layout.

The `Janus-OS` root workspace manifest still declares its Cargo repository URL as `https://github.com/Wizardwalt/Janus-Omega-OS`. Its README's architecture tree is also headed `Janus-Omega-OS/`. These are concrete signs that Janus-OS retains the Omega project identity/lineage in its current code and documentation.

### Material implementation divergence

The default branch of `Janus-OS` documents and contains a split-service architecture:

- `janus-runtime` is a distinct protected runtime daemon.
- `janus-web` serves the UI and proxies to the runtime.
- The launcher builds and starts both binaries and waits for the runtime health endpoint.
- Production Lua plugins execute in a fresh sandbox with an instruction limit; only logging is currently exposed as a host capability.
- The legacy WebSocket runner is explicitly demo-only; simulated shell, ADB, and hardware results are not real operations.
- Hardware status distinguishes an actual configured serial connection from unsupported GPIO, cellular, RF, and sensor adapters.

The default branch of `Janus-Omega-OS` still documents the older single-web-server / WebSocket Lua runner. Its README explicitly lists simulated shell and ADB output, success-returning action stubs, and automatic unknown-global stubs. It also documents the ArchISO boot experience and Pandora Mk.1 / Omega / Titan hardware concepts.

The checked `janus-runtime/src/main.rs` files differ materially: the Janus-OS version includes offline backup restore, configurable hardware disabling, and runtime backup initialization; the Omega version is the simpler earlier daemon startup path.

### What is not proven yet

- Git commit ancestry between the two repositories has not been established.
- No full recursive tree comparison or complete file-by-file diff has been performed.
- The current Omega default branch's ArchISO build, hardware design assets, and latest active branches have not yet been exhaustively inventoried.
- A matching branch name in both repositories does not prove that the branch tips or histories are identical.

## Operating decision for Stage 1

1. Make new protected-runtime fixes in `Wizardwalt/Janus-OS`.
2. Keep Janus-Omega-OS intact as its own repository and preserve its original project name and historical material.
3. Do not merge, rename, delete, force-push, or bulk-copy either repository as part of this audit.
4. Inventory Omega's ArchISO and Pandora hardware assets separately. Migrate only individually reviewed assets with provenance and tests.
5. After a full tree and ancestry comparison, record a final ownership decision in both repositories.

## Required next verification

- Compare commit ancestry using Git object history or a local clone of both repositories.
- Generate recursive file inventories for both default branches and relevant active branches.
- Classify each unique file/path as shared, newer in Janus-OS, unique to Janus-Omega-OS, or obsolete/unclear.
- Build the ArchISO independently and verify boot in a VM before claiming bootability.
- Preserve any unique Omega documentation, hardware design, or lore before considering migration.

## Evidence labels

- **Implemented and tested:** only where the repository contains the implementation and a relevant passing automated test/build.
- **Implemented but unverified:** code exists but the required integration/device test has not been performed.
- **Simulated/demo:** explicitly labelled mock behavior.
- **Stub/placeholder:** code returns canned values or explicitly reports that an adapter is not implemented.
- **Planned:** described as future work without a working implementation.

This report deliberately distinguishes product lineage from Git ancestry and avoids treating README claims as test evidence.
