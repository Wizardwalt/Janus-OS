# ARIA Completion Roadmap

## Purpose

Complete ARIA as JanusOS's persistent, learning-capable, multi-device AI companion. This roadmap separates working behavior from interface simulations and keeps the original project vision intact while the historical LM Arena conversations are being recovered.

This document records the current repository evidence and implementation order. It does not claim that planned capabilities already exist. The security baseline is defined in [ARIA_SECURITY_ARCHITECTURE.md](ARIA_SECURITY_ARCHITECTURE.md).

## Current verified state on `repair/janus-os-full-repair`

- `web/aria3d.js`, `web/aria_profile.js`, and `web/aria_setup.js` provide a 3D avatar, a first-run customization wizard, and a browser-local profile/modification store.
- `web/app.js` sends chat messages over the legacy `/ws` channel using `aria_chat`.
- `janus-web/src/main.rs` handles `aria_chat` with the scripted `aria_respond()` function. It explicitly says that a live model and persistent learning are not connected.
- The WebSocket sends periodic scripted thoughts. Those messages are not model-generated cognition or live telemetry.
- The production `janus-runtime` already has authentication, an organization-scoped SQLite state API, and an authenticated browser client in `web/runtime_api.js`. The current ARIA chat path does not use that API for memory.
- `docs/ARIA_PORTABILITY_AND_RECOVERY.md` defines the intended portable identity, memory journal, replication, failover, and restore requirements. It explicitly describes an architecture requirement, not an implemented feature.
- No model provider is currently wired into the ARIA chat path. No original JANUS or ARIA acronym expansion is established by the inspected source; do not invent one.

## Implementation principles

1. Never describe scripted text, mock output, or a UI animation as model reasoning, learning, hardware telemetry, or a completed operation.
2. Keep ARIA local-first: the companion must remain useful when the Monolith or network is unavailable.
3. Make identity, memories, learning records, and approved preferences portable; keep host secrets, device handles, and credentials out of the portable bundle.
4. Make memory writes durable before acknowledging them. Show pending replication separately from replicated state.
5. Treat learning as traceable updates to memory, preferences, and knowledge—not uncontrolled self-modification of executable code or permissions.
6. Keep model providers replaceable through a provider-neutral adapter. Never put provider API keys in browser JavaScript or localStorage.
7. Require explicit, scoped approval for privileged, destructive, privacy-sensitive, or externally consequential actions. Record an audit event and provide rollback where possible.
8. Preserve user-approved identity and historical source material. Do not retroactively assign an acronym or persona detail without evidence.
9. The purchaser is the device Owner and has full authority over documented, configurable security policies through a strongly authenticated Owner Console. Owner overrides must be explicit, scoped, and audited; they do not give ARIA or plugins Owner authority or remove core integrity guarantees.

## Ordered work

### Phase 0 — Characterize and contract
- Map every ARIA entry point, UI event, WebSocket message, runtime API, persistence layer, and test.
- Confirm the active repair branch and working-tree state before every code change.
- Define versioned data contracts for identity, conversation events, memory records, model settings, skills, and sync metadata.
- Keep the original LM Arena export as source evidence when it becomes available.

**Exit criteria:** a reviewed dependency map and tests for the existing chat and profile behavior.

### Phase 1 — Real conversational backend
- Replace the scripted response path with a server-side model-provider interface.
- Support a configurable OpenAI-compatible chat endpoint and a local-model endpoint through the same adapter contract.
- Store secrets only in the runtime's protected configuration/secret mechanism; never expose them to the browser.
- Add request timeouts, bounded input/output, cancellation, clear provider-unavailable errors, and model/provider status.
- Preserve a deterministic offline fallback that clearly identifies itself as fallback text, not AI output.

**Exit criteria:** a real configured model can answer through the ARIA UI; invalid credentials, unavailable providers, timeouts, and offline operation are handled honestly and tested.

### Phase 2 — Durable conversation and memory
- Add user-scoped persistent conversation events and memory records in SQLite.
- Add retrieval of relevant prior context with source/event references.
- Support user corrections, explicit remember/forget controls, export, and deletion semantics.
- Separate raw conversation history from extracted long-term memories; never silently turn every utterance into a permanent fact.
- Add retention limits, database migrations, access checks, and audit coverage.

**Exit criteria:** conversation survives app restarts, remains isolated per user, can be exported, and memory retrieval can show which stored records informed an answer.

### Phase 3 — Learning and knowledge management
- Add an auditable learning pipeline: propose memory/knowledge updates, show evidence and confidence, and accept or reject them.
- Track source, timestamp, version, and correction history for each learned record.
- Support user-provided documents and Janus module documentation as searchable knowledge sources.
- Do not permit learning to rewrite runtime code, grant new capabilities, or weaken security controls.

**Exit criteria:** accepted learning updates persist and are retrievable; rejected or corrected items remain traceable; executable code and permissions are unchanged.

### Owner-control implementation progress

- Added SQLite persistence for Monolith/Pip-Boy ownership records and per-device configurable policy JSON.
- Added authenticated runtime endpoints to register a device with password re-entry, list only the caller's registered devices, and update allowlisted boolean settings after password re-entry.
- Added a browser Owner Security Console and client methods for those endpoints.
- Owner registration is currently bound to the initializing OrganizationAdmin account. This is an account enrollment slice, not proof of physical device possession.
- The stored policy settings are not yet wired into the capability broker or device services; do not treat them as active enforcement.
- Remaining: cryptographic device enrollment/attestation, purchaser-facing Owner Console, enforcement at each affected operation, break-glass recovery and ownership transfer, and automated negative/positive tests.

### Phase 4 — Skills and controlled tools
- Add a registry of typed ARIA tools with declared inputs, outputs, required permissions, and audit events.
- Route hardware and Janus module requests through existing authenticated, capability-gated runtime services.
- Distinguish planning from execution and report results only after a service confirms them.
- Add user approval gates for sensitive actions and a read-only default.

**Exit criteria:** each enabled tool has permission tests, negative tests, audit evidence, and truthful UI status.

### Phase 5 — Voice and presence
- Add speech-to-text and text-to-speech adapters, configurable voice and audio controls, interruption, and accessibility options.
- Keep avatar emotion/presence driven by explicit response metadata rather than pretending the avatar has independently measured feelings.
- Make voice optional and usable offline where a supported local engine is installed.

**Exit criteria:** voice can be enabled/disabled, interrupted, and used without breaking text chat; no microphone capture occurs without visible user action.

### Phase 6 — Portable identity, backup, and Monolith/Pip-Boy sync
- Implement signed, versioned export/import bundles for identity, approved preferences, memory, conversation references, and skill manifests.
- Encrypt backup contents and verify signatures and hashes before restore.
- Add an append-only event journal, checkpoints, resumable idempotent replication, and fencing against stale writers.
- Keep host-specific secrets and device bindings out of the portable bundle; re-enroll them on a replacement host.
- Implement failover and restore drills for loss of Pip-Boy, Monolith, and both primary systems.

**Exit criteria:** a clean second Janus host can restore all acknowledged ARIA state from a verified independent backup and safely resume synchronization.

### Phase 7 — Hardening and release
- Add unit, integration, API contract, browser, security, migration, backup/restore, and failure-injection tests.
- Verify Android-sized layouts and touch interaction.
- Document installation, provider configuration, secret management, offline behavior, backup, restore, and troubleshooting.
- Remove or clearly label all obsolete scripted/demo ARIA paths.

**Exit criteria:** release checklist passes on a clean build and a second-device restore; every claimed capability has reproducible evidence.

## Decisions still intentionally open

- Original expansions of JANUS and ARIA: recover from historical conversations; do not guess.
- Default model provider and deployment target: choose after comparing hosted API cost/privacy with local model hardware requirements.
- Long-term memory policy and retention: require explicit user approval before enabling automatic memory extraction.
- Exact Monolith transport and replication topology: confirm actual hardware/network design before implementing failover.

## Immediate next engineering slice

Characterize the authentication and runtime boundary, then implement the server-side model adapter without exposing credentials to the browser. Follow it with user-scoped persistent conversation storage. Do not wire ARIA memory into the existing organization-wide generic state key: that state is organization-scoped, not an appropriate private per-user memory boundary.
