# Janus Device Service Center

**Status:** initial read-only dashboard increment in development; not a production device-servicing suite.

## Product intent

Add a first-class device-servicing environment to Janus OS, using ChimeraTool's operator-facing workflow as inspiration while retaining Janus OS's existing identity, launcher, runtime, plugin system, authorization model, and web dashboard. This is an additive subsystem, not a replacement or redesign.

## Existing integration points verified

- The current launcher is served from `web/index.html` and uses `web/app.js`.
- The authenticated browser client is `web/runtime_api.js`.
- The Rust runtime already exposes authenticated, read-only inventory routes: `GET /hardware/status`, `GET /hardware/usb/devices`, `GET /hardware/serial/ports`, `GET /hardware/thermal/sensors`, and `GET /hardware/memory`.
- `janus-runtime/src/hardware.rs` provides USB sysfs inventory, serial-port enumeration, thermal readings, and memory inventory. USB serial numbers are deliberately excluded and discovery does not open or probe devices.
- The protected plugin sandbox does not currently grant shell, ADB, filesystem, network, or hardware capabilities to plugins. The dashboard must not bypass that boundary.

## First increment

This increment adds a launcher tile and dedicated Device Service Center view. It consumes only the existing authenticated hardware status, USB inventory, and serial inventory endpoints. It presents real returned inventory or explicit unavailable/error states; it never invents devices or treats demo output as real.

No device-changing action is included. It does not invoke ADB, Fastboot, EDL, bootloader unlock, firmware flashing, factory reset, FRP operations, or hardware programmer commands.

## Target dashboard areas

1. Overview — connected device count, adapter readiness, active operation state, recent operation history.
2. Device inventory — USB, serial, Android/ADB, Fastboot, recovery and supported transport identification, with explicit source and confidence.
3. Device profile — manufacturer/model/chipset matching against a versioned support catalogue.
4. Diagnostics — read-only device and host diagnostics with reproducible reports.
5. Firmware & resources — signed/versioned firmware, driver and tool metadata; provenance, hashes and compatibility checks.
6. Service workflows — only after an adapter exists, an operation registry is reviewed, and authorization/confirmation/audit controls are implemented.
7. Operation center — queued/running/completed/failed/cancelled states, progress, logs with secrets redacted, and exportable reports.
8. Aria assistance — explain detected state and recommend documented next steps; no autonomous destructive actions.

## Architecture boundaries

- Keep the subsystem modular and attach it to existing Janus launcher navigation.
- Frontend calls the authenticated same-origin runtime API; no direct browser-to-device shell bridge.
- Add runtime services behind explicit typed interfaces and capability checks; do not put device protocols in frontend code.
- Use least privilege and separate read-only discovery from device-changing operations.
- Every future write/service operation must require an authenticated operator, an explicitly selected device, scoped authorization/engagement, a clear impact summary, second confirmation, cancellation/timeout handling, and an immutable audit event.
- Do not implement bypasses for account locks, anti-theft protections, MDM, or ownership controls. Support only lawful owner-authorized repair and recovery.
- Avoid storing device serial numbers or other unique identifiers unless the operator explicitly enables a justified, protected inventory feature.

## Phased delivery

- **Phase 0 — existing capability audit:** verified current authenticated USB/serial/readiness endpoints and capability restrictions.
- **Phase 1 — read-only dashboard:** launcher integration, live inventory, clear sign-in/error states. This PR implements this first slice.
- **Phase 2 — model catalogue:** curated, versioned device and chipset support metadata; no claimed support without test evidence.
- **Phase 3 — read-only protocol detection:** separately reviewed ADB/Fastboot transport discovery with explicit permission and no writes.
- **Phase 4 — diagnostics and operation records:** typed job model, progress/cancellation, redaction, audit.
- **Phase 5 — authorized service adapters:** one adapter at a time, test fixtures and hardware-in-loop acceptance before enabling device-changing workflows.

## Acceptance criteria

- Existing Janus launcher and module navigation remain unchanged.
- Device Service Center is a separate launcher destination.
- Signed-out users see a sign-in requirement, not demo inventory.
- Signed-in users see only live runtime USB/serial/adapter results.
- Endpoint failures are shown as unavailable; partial results do not masquerade as complete scans.
- No device-changing commands or privileged capabilities are introduced.
