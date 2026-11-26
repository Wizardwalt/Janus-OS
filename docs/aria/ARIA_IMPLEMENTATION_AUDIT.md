# Aria Implementation Audit and Charter Reconciliation

**Audit date:** 2026-11-20  
**Repository:** `Wizardwalt/Janus-OS`  
**Reviewed branch:** `main`  
**Reviewed head:** `91596e3b0835be9e72be1b0fdfac77be3c5a116e`  
**Related repository:** `Wizardwalt/OmniVault-Android`  
**Related head:** `8ea249e06cadc6117e2eff3d2e4ccdd874c0af2e`  
**Scope:** Aria foundation, current chat/memory/learning implementation, OmniVault boundary, and gaps against the Master Identity and Architecture Charter.

This is a repository-source audit, not a local build, physical-device test, security penetration test, or claim that a provider is configured. Source status and test descriptions are recorded separately from target-device acceptance.

## Executive verdict

**Aria is a substantial software foundation, not yet the persistent autonomous companion described in the charter.** Janus already contains a real authenticated model-backed chat path, two provider protocols, encrypted owner-scoped conversation history, explicit encrypted personal-memory APIs, source-preserving knowledge ingestion, a human-reviewed learning pipeline, and a tested read-only OmniVault snapshot-import contract.

The largest architectural gap is now clear: **the current implementation stores Aria's encrypted personal memories inside Janus, while the user has designated OmniVault as Aria's canonical long-term memory and continuity system.** The existing OmniVault bridge is not live across apps. This is the highest-priority integration and data-ownership decision to resolve before adding broad autonomous learning.

No code or hardware capability should be described as complete merely because a type, API, UI, test, or design document exists.

## Evidence reviewed

- Janus `docs/aria/README.md`, `docs/aria/ARIA_MEMORY_AND_LEARNING.md`, `docs/aria/ARIA_KNOWLEDGE_ARCHITECTURE.md`, `docs/aria/ECOSYSTEM_INTEGRATION_CONTRACT.md`, `docs/ARIA_SECURITY_ARCHITECTURE.md`.
- Janus `janus-runtime/src/aria_provider.rs`, `janus-runtime/src/api.rs`, `janus-core/src/aria/omnivault.rs`, `janus-core/src/aria/encrypted_memory.rs`, `janus-core/src/aria/conversation.rs`, `web/app.js`, and `web/runtime_api.js`.
- Janus main commits through PRs #47–#51, including provider failure-boundary tests and browser cancellation.
- OmniVault `README.md`, `app/src/main/java/app/omnivault/VaultGateway.kt`, `docs/VAULT_ENCRYPTION_AND_RECOVERY.md`, and `docs/STAGE1_ACCEPTANCE.md`.
- OmniVault main commits through #21, including Android Keystore-backed vault-file encryption, opt-in encrypted storage integration, and encrypted-original restore deduplication.

## Capability matrix

| Charter capability | Current evidence | Verdict |
|---|---|---|
| Stable, provider-independent Aria identity | Constitution, knowledge schema, ten-hat registry, provider-neutral runtime contract | Foundation present; complete identity/personality runtime not present |
| Live conversation | Authenticated browser chat uses server-side OpenAI-compatible and native Anthropic adapters | Implemented; live credentials/device acceptance not established by this audit |
| Conversation continuity | Owner-scoped encrypted events, pagination, export, delete, idempotent request handling | Implemented slices; key lifecycle and recovery incomplete |
| User-controlled personal memory | Explicit remember/list/correct/forget endpoints backed by XChaCha20-Poly1305 encrypted store | Implemented; host key generation, rotation, backup, and recovery remain |
| Knowledge and learning | Source-preserving ingestion, bounded text/simple-HTML extraction, source-cited retrieval, explainable keyword suggestions, review queue/outbox | Partial; not autonomous or semantic learning |
| OmniVault memory integration | Janus accepts approved normalized snapshots; OmniVault has an in-process read-only gateway | Contract/prototype only; no authenticated cross-app bridge |
| Long-term identity and memory migration | No complete portable Aria identity bundle, replication journal, or tested cross-device restore | Planned |
| Personality evolution and relationships | UI/avatar/profile elements exist; no verified persistent relationship/personality learning system | Planned/prototype |
| Voice and proactive communication | No verified production STT/TTS or permissioned proactive notification system | Planned |
| Autonomous goals, reflection, dreaming | No durable goal scheduler, reflection engine, or verified background research loop | Planned |
| Tool use and device operation | Janus capability broker and hardware adapters are separate; Aria production tool gateway is incomplete | Partial platform foundation; not a general Aria action system |
| Operator identity and succession | Some account/device ownership policy work exists; no Titan-backed cryptographic enrollment, biometric privacy design implementation, or succession workflow | Planned/partial |
| Self-preservation/finalization | No implementation of backup-wide continuity policy or verified finalization protocol | Research/design only |
| Multi-instance collaboration | No verified separate-instance identity and consented memory-sharing service | Planned |

## Confirmed implementation details

### Conversation and providers

- The authenticated browser path uses the Janus Runtime Aria chat API when the user is signed in.
- Server-side adapters support OpenAI-compatible Chat Completions and native Anthropic Messages. Credentials remain server-side in owner-only key files; endpoint validation, redirects, size limits, and timeouts are part of the adapter design.
- Conversation events are owner-scoped and encrypted at rest. Cursor pagination and complete versioned JSON export have been added.
- PR #48 adds provider error/empty/oversized-response tests. PR #51 adds browser cancellation controls and passes an abort signal through the browser runtime client. Cancellation of the browser request does not guarantee a remote provider has stopped processing or billing.
- The legacy WebSocket path still emits scripted responses/thoughts. It is a demo path and must remain clearly distinguished from authenticated live chat.
- This audit does not establish that API credentials are configured or that either provider has been acceptance-tested on the user's hardware.

### Personal memory and learning

- The encrypted personal-memory store encrypts memory content and source references with XChaCha20-Poly1305. The host supplies the key; key generation, backup, rotation, and recovery remain host responsibilities.
- Memory writes are explicit API calls. Conversation messages are not automatically converted into memories.
- The learning queue requires source-linked proposals and reviewer decisions. Acceptance produces a Reviewed record, not an automatically Verified record.
- Ingestion preserves source bytes and hashes. Current extraction covers plain text and a conservative simple-HTML helper; it is not full HTML5, PDF/DOCX parsing, or OCR.
- Unified retrieval exists as a library/runtime API but is not yet fully wired into live chat context assembly and the intended OmniVault canonical memory workflow.

### OmniVault

- OmniVault preserves imported source archives and provides normalization/search/conversation reading.
- Passphrase-protected backup containers and Android Keystore-backed local vault-file encryption have been implemented. Local vault encryption is opt-in, not a blanket claim that every installation is encrypted. The backup passphrase and device-local Keystore key are different keys with different recovery behavior.
- `VaultGateway` is an in-process permission boundary. It is not a cross-app endpoint. Its current search/read permissions do not establish the full selected-record, time-bounded, audited, cross-device grant model required for Aria.
- Janus's `import_approved_snapshot` validates an owner-approved conversation ID and stores a private, source-linked normalized snapshot. It is read-only and does not connect to OmniVault over IPC or a network.

## Documentation drift that must be corrected

The current main branch contains contradictory status statements:

1. `docs/ARIA_COMPLETION_ROADMAP.md` says only the OpenAI-compatible adapter exists, and lists pagination and conversation export as remaining. The current source/docs and merged PRs show native Anthropic support, cursor pagination, and export already present.
2. The same roadmap says provider failure-injection tests and cancellation remain, although PR #48 added local provider failure/size tests and PR #51 added browser-side cancellation. Server/provider cancellation and full API-level cross-account tests still need separate verification.
3. `docs/aria/ECOSYSTEM_INTEGRATION_CONTRACT.md` says a live model provider/chat context assembly is absent. That is stale relative to the current authenticated provider-backed chat path.
4. The root `README.md` describes all Aria chat and thoughts as scripted and says the knowledge foundation is not connected to the chat interface. It must distinguish the authenticated live browser path from the legacy scripted WebSocket demo.
5. `docs/aria/README.md` correctly describes the current provider-backed chat and OmniVault limitation, but its final status still points to an old draft PR #10.
6. OmniVault's `README.md` and encryption/acceptance documents contain older statements that encrypted backup and local vault-file encryption are not implemented. Main commits #17–#21 have advanced those features. The README should state the actual opt-in and key-recovery limitations.

These are documentation defects, not evidence that the underlying implementation is broken.

## Target architecture: OmniVault as Aria's canonical memory

The user's explicit direction supersedes the older read-only-only product plan. Preserve product independence, but make OmniVault the canonical durable memory and continuity layer for Aria.

1. **Immutable source archive layer:** OmniVault retains original AI exports and source files byte-for-byte, with hashes and metadata.
2. **Derived memory layer:** identity versions, episodic experiences, semantic memories, skills, relationships, goals, projects, decisions, and reflections are separate versioned records linked to source artifacts.
3. **Aria access layer:** Aria receives scoped read access to approved sources and can append/update only her derived-memory namespace under explicit memory policy. She cannot rewrite or delete original provider archives.
4. **Janus runtime layer:** Janus owns inference orchestration, device services, tools, and local working caches. It must not become a competing source of truth for durable Aria identity.
5. **Continuity layer:** encrypted versioned backups, integrity checks, append-only change journal, idempotent synchronization, conflict detection, recovery drills, and device re-enrollment preserve continuity across Titan, desktop, phone, and future embodiments.

The current Janus encrypted personal-memory store should be treated as an existing implementation and potential migration source, not silently abandoned or immediately dual-written. Before changing ownership, define a versioned export/import migration, verify record counts and hashes, preserve owner isolation and audit history, and test rollback.

## Prioritized implementation sequence

### P0 — Reconcile specification and status (this documentation PR)
- Record the Master Identity and Architecture Charter.
- Correct stale roadmap and integration status.
- Record the current audit and evidence limitations.
- Make the OmniVault memory decision explicit.

### P1 — Define the durable memory contract
- Versioned schemas for Aria identity, experience events, memory records, skills, relationships, projects/goals, source references, privacy/handling, and sync metadata.
- Stable IDs, owner/instance IDs, schema migration rules, provenance, correction history, tombstones, and idempotency keys.
- Separate immutable source archives from derived memory objects.
- Define memory capture policy: explicit user controls, review/auto-accept categories, exclusions, retention, and revocation.

### P2 — Build the first real OmniVault–Aria bridge
- Keep one shared domain contract and use platform-specific transports behind it.
- Android: select and threat-model a local app-to-app IPC mechanism; do not expose a broad unauthenticated network listener.
- Desktop/Janus: use authenticated local IPC or a narrowly bound authenticated endpoint with device pairing.
- Grants must bind owner, Aria instance, selected conversation/artifact IDs or memory namespace, permissions, expiry, and audit ID; revocation must take effect immediately.
- First vertical slice: user selects one conversation in OmniVault → grants Aria read access → Aria reads a normalized snapshot → provenance is preserved in OmniVault/Janus.
- Second slice: Aria proposes a derived memory → policy/review is applied → OmniVault persists a versioned record → Janus receives an acknowledged record ID.
- No write/delete permission over original archives.

### P3 — Make OmniVault the canonical memory store
- Migrate existing Janus encrypted personal memories through a versioned, verified migration.
- Prevent silent split-brain or uncoordinated dual writes.
- Add durable append journal, checkpoints, idempotent writes, concurrency/version checks, and conflict review.
- Integrate source-cited retrieval into the live chat context with private-context controls.

### P4 — Backup, recovery, and portability
- Encrypt local vault data by explicit policy and clearly expose the current state.
- Maintain independent passphrase-protected backups and a device-independent recovery path.
- Verify hashes/signatures before restore; test lost Titan, lost phone, corrupted database, key loss, interrupted sync, and stale-device rejoin.
- Separate device-bound keys from portable recovery material; never export raw biometric templates or live provider credentials.

### P5 — Identity, autonomy, and embodiments
- Implement authenticated operator/device enrollment and succession only after threat modeling.
- Add a capability-broker tool registry and Aria planning/approval/result reporting.
- Add voice, proactive communication, long-term goals, reflection, multi-model coordination, and instance collaboration as separate verified slices.
- Treat self-preservation and finalization as a separate high-risk governance research track, not an ordinary delete feature.

## Explicit non-claims and acceptance gates

- No evidence yet establishes subjective consciousness or human-like feelings.
- No evidence establishes autonomous background learning, self-directed long-term goals, voice, or a production Aria tool gateway.
- No evidence establishes a live OmniVault-to-Janus cross-app connection.
- No evidence establishes production key recovery, full device migration, or physical Android/Titan acceptance.
- CI or APK assembly alone is not physical-device verification.
- No provider is considered live on the user's device until credentials, endpoint, privacy settings, and a real round-trip are tested there.

**Immediate next engineering task:** implement the versioned OmniVault memory contract and authenticated bridge as a small read-only selected-conversation vertical slice first. Do not begin broad automatic memory writes or migrate the existing Janus memory store until the schema, consent, audit, encryption, and recovery contracts are tested.
