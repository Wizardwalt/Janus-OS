# ARIA Completion Roadmap

## Purpose

Complete ARIA as JanusOS's persistent, learning-capable, multi-device AI companion. This roadmap separates working behavior from interface simulations and keeps the original project vision intact while the historical LM Arena conversations are being recovered.

This document records the current repository evidence and implementation order. It does not claim that planned capabilities already exist. The security baseline is defined in [ARIA_SECURITY_ARCHITECTURE.md](ARIA_SECURITY_ARCHITECTURE.md).

## Current verified state on `main` — 2026-11-20

- `web/app.js` uses the authenticated `window.janusRuntime.ariaChat` API for live chat when a user is signed in. It loads owner-scoped conversation history, exposes a user-controlled private-context toggle, and labels provider/storage readiness. The 3D avatar remains a presentation layer, not evidence of independent cognition.
- `janus-runtime/src/aria_provider.rs` contains a server-side asynchronous provider contract, an OpenAI-compatible HTTP adapter, and a native Anthropic Messages API adapter. `JANUS_ARIA_PROVIDER_KIND` selects the protocol; each adapter reads its credential from an owner-only key file, disables redirects, restricts cleartext HTTP to loopback, and bounds request count, response bytes, output length, and request time. Local model servers can use the OpenAI-compatible adapter when they implement that protocol; other protocols still need their own adapter.
- `POST /aria/chat` persists the authenticated user's user/assistant events, uses a request ID for retry idempotency, and optionally assembles private context only when the user explicitly enables it. Retrieved reference material is marked as untrusted data; knowledge IDs/epistemic status/source URIs, source artifact IDs/chunk ordinals/hashes, and personal-memory IDs/source refs are preserved in model context.
- `ConversationStore` is owner-scoped and encrypts event payloads at rest with XChaCha20-Poly1305 using the host-supplied Aria data key. Tests cover owner isolation, reopen persistence, plaintext absence, and wrong-key failure. The data-key file and recovery lifecycle still need operational hardening.
- `janus-runtime/src/api.rs` exposes authenticated provider readiness, conversation list/read/append/delete, and live chat routes. Conversation events support owner-scoped cursor pagination via `after_event_id`. The browser client does not receive the provider API key.
- The legacy WebSocket path in `janus-web/src/main.rs` still uses scripted `aria_respond()` output and periodic scripted thoughts. It is a demo path, not the live provider-backed chat path. Signed-out UI explicitly labels its remaining chat as scripted demo mode.
- The current provider contract is transport-neutral at the runtime boundary, but only the OpenAI-compatible adapter is implemented. Streaming, arbitrary vendor/local adapters, API-level failure-injection tests, conversation export/retention, and full key rotation/recovery remain incomplete.
- No original JANUS or ARIA acronym expansion is established by the inspected source; do not invent one.

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
**Status: partially implemented.**
- The authenticated browser UI now uses the runtime's server-side provider adapter for live chat; the legacy WebSocket remains scripted demo mode.
- A provider-neutral async trait is now the runtime boundary. The implemented adapter speaks the OpenAI-compatible chat-completions protocol; additional provider protocols and streaming are not yet implemented.
- Credentials stay server-side. The current adapter validates HTTPS/loopback endpoints, rejects redirects, bounds inputs/outputs, and applies connection/request timeouts.
- The runtime reports provider readiness and returns explicit unavailable/error responses rather than fabricating model output.
- Remaining: provider-specific adapters, streaming, cancellation across provider HTTP requests, stronger API-level failure-injection tests, and a clearly identified offline fallback.

**Exit criteria:** each adapter has contract tests, live UI round-trips are tested against a local fixture provider, and unavailable/invalid/timeout states remain truthful.

### Phase 2 — Durable conversation and memory
**Status: partially implemented.**
- Encrypted, owner-scoped conversation events and authenticated list/read/append/delete routes are connected to the live chat UI.
- Private-memory and knowledge retrieval are opt-in per chat request; retrieved material carries source references and is explicitly treated as untrusted reference data.
- Explicit remember, correct, forget, and reviewed learning-proposal routes exist in the authenticated runtime.
- Remaining: conversation export, cursor pagination for long histories, retention/expiry policy, key rotation and recovery, and API-level cross-account/failure-injection coverage.

**Exit criteria:** exports and deletion are verified across restarts, long histories paginate without omission, and key-loss/recovery behavior is documented and tested.

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
- Extracted owner-device identity and policy-merge validation into tested helpers, including rejection of malformed IDs, unknown device types, unknown policy keys, and non-boolean values; verified partial updates preserve unchanged settings.
- Tightened device-policy updates to organization administrators only, added fresh password re-authentication tied to the exact account ID, and fixed audit metadata to record the submitted setting names.
- Added SQLite persistence coverage for owner isolation, cross-owner read/update denial, and prevention of device-ID reassignment.
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

Add contract-level API tests for authenticated live chat, cross-account conversation access denial, provider failure, and idempotent retries. Then implement at least one additional adapter protocol (not merely another OpenAI-compatible endpoint), followed by streaming/cancellation and export/retention/key-rotation acceptance. Do not wire Aria memory into the organization-wide generic state key: that state is organization-scoped, not an appropriate private per-user memory boundary.
