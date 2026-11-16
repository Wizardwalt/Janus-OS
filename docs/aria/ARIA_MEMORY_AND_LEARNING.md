# Aria Memory and Learning System

**Status:** Stage 2 implementation in progress. This document distinguishes working library behavior from host integrations that are not yet present.

## Implemented source-preservation layer

`janus_core::aria::ingestion` provides a durable SQLite artifact store for UTF-8 text:

- preserves the exact imported byte sequence and records its SHA-256 digest;
- creates deterministic overlapping chunks with byte offsets into the original;
- verifies the digest when original bytes are read;
- keeps artifact metadata and chunks in one transaction;
- rejects duplicate artifact IDs, invalid UTF-8, invalid classifications, invalid expiry, and invalid chunk parameters;
- filters expired, private, sensitive, and project-scoped artifacts using a scope supplied by the host.

This is a preservation and text-indexing layer, not a PDF/DOCX/HTML parser. Format-specific extraction adapters are still required. Binary originals can be preserved by a future binary artifact API; this initial text API deliberately rejects invalid UTF-8 rather than corrupting it.


## Implemented extraction and source-cited retrieval

- `extract_plain_text` validates UTF-8 and preserves text exactly.
- `extract_simple_html` provides conservative extraction for simple exported HTML: strips tags, skips script/style bodies, decodes common/numeric entities, and returns normalized text. It is explicitly not a complete HTML5 parser or a sanitizer.
- `AriaIngestionStore::search_chunks` searches extracted chunks within a supplied access scope and returns source passages with artifact ID, SHA-256, title, ordinal, and exact byte offsets.
- `LearningProposal::from_passage` converts a retrieved passage into an unverified, source-linked proposal without dropping the original artifact hash or locator. The proposal then enters the existing human review queue.

This closes the basic text → preserve → chunk → retrieve with citation → propose → human review library path. The caller still chooses the initial hat labels and source type; this is not automatic AI classification.

## Implemented explainable Multicolored Hat suggestions

`janus_core::aria::classification::suggest_hats` provides a deterministic keyword baseline:

- returns each suggested hat with the exact matched terms and a simple match count;
- suggests Rainbow when two or more non-White domains match;
- suggests White for documentation, standards, source, and evidence terms;
- returns no match rather than inventing a label.

The match count is not a calibrated probability or confidence score. The function is advisory: a user or reviewer must select/correct the final hat labels. It does not automatically write knowledge records or grant capabilities. A future semantic/model classifier can be added behind the same review boundary.

## Implemented human-review learning queue

`janus_core::aria::learning` provides a durable proposal and review queue:

1. A caller creates a `LearningProposal` containing a proposed `KnowledgeRecord` and a source locator.
2. The proposal must cite the exact artifact ID and SHA-256 in its provenance.
3. Proposals cannot enter the queue already marked `Verified`.
4. Pending proposals can be listed for a review interface.
5. A reviewer submits an immutable accept/reject receipt with reviewer ID, timestamp, and notes.
6. Rejection returns no knowledge record. Acceptance returns the proposed record with epistemic status `Reviewed`; it does not automatically mark it verified.
7. The host is responsible for authorizing the reviewer. Accepted records are written to a durable outbox in the same transaction as the immutable review receipt; `LearningQueue::persist_pending_for_project` transfers only the authenticated organization's records into `KnowledgeStore` and acknowledges only after persistence.

The queue is a library component, not an authenticated API. Hosts must enforce user identity, artifact access, reviewer authorization, and project/handling scopes before calling it. The proposal queue and knowledge store remain separate SQLite connections (on the Aria database file), so this is a recoverable at-least-once outbox hand-off, not a single SQLite transaction spanning review and knowledge insertion. If the process stops after knowledge insertion but before outbox acknowledgement, an identical record is recognized on retry. Conflicting existing records stop the hand-off for operator review. The host must report full learning persistence only after the outbox is drained.


## Implemented explicit personal-memory store

`janus_core::aria::memory` provides a separate SQLite store for explicit personal memories:

- Each memory has an owner ID; reads, corrections, lists, and forget operations require the matching owner ID.
- New memories must be explicitly written by the host. The store does not automatically turn conversation messages into memories.
- Corrections increment a version and retain an audit event containing the prior content digest.
- Forgetting deletes the stored content and source references and clears prior content digests from the audit trail, retaining only minimal event metadata.
- Expired memories are hidden from reads and lists and cannot be corrected.
- Unit tests cover owner isolation, correction history, forget behavior, expiry, and search beyond the most recently updated unmatched memories.

The owner ID is an authorization input supplied by the host, not proof of identity. The host must authenticate the user before calling the store. The original `MemoryStore` is a plaintext development/test component and must not be used for sensitive production memories.

`janus_core::aria::EncryptedMemoryStore` is now available. It encrypts the complete serialized memory (content and source references) with XChaCha20-Poly1305 using a fresh 192-bit nonce for every write. Owner ID and indexing metadata remain visible to SQLite; authenticated decryption verifies that the encrypted record's owner and memory IDs match those index fields. Wrong keys and modified ciphertext fail authentication. Correction uses optimistic version checks; forget deletes the encrypted row and clears historical content digests from its audit trail.

The Janus Runtime now uses this encrypted store for its memory API only when `JANUS_ARIA_MEMORY_KEY_FILE` points to a regular, non-symlink file containing exactly 64 hexadecimal characters (32 key bytes). On Unix, the key file must have owner-only permissions (0600). The key is never written to the database. If no key file is configured, memory endpoints return service unavailable and unified retrieval simply omits personal memories. The host remains responsible for secure key generation, backup, access control, rotation, and recovery; do not place the key beside the database or in source control.

Authenticated, explicit user-action endpoints are available: `POST /aria/memory` (remember), `GET /aria/memory` (list), `POST /aria/memory/{id}/correct`, and `DELETE /aria/memory/{id}` (forget). No conversation is automatically converted into a personal memory.

## Authenticated Janus Runtime API integration

The runtime now opens a separate Aria SQLite database beside the configured Janus runtime database and exposes these bearer-session-authenticated endpoints:

- `POST /aria/artifacts/text` — import up to 1 MiB of UTF-8 text as a private artifact owned by the authenticated account.
- `GET /aria/search?q=...` — unified search over unscoped public/internal knowledge and artifacts, the authenticated account's own private project, and that account's encrypted personal memories. Unscoped private/sensitive records are never included.
- `POST /aria/proposals` — create a source-linked proposal from a passage in an artifact the caller owns; the proposal is explicitly submitted to the caller's organization review queue.
- `GET /aria/proposals/pending` — list pending proposals scoped to the authenticated organization; Reviewer or OrganizationAdmin role required.
- `POST /aria/proposals/{proposal_id}/review` — accept or reject an organization-scoped proposal; Reviewer or OrganizationAdmin role required. Accepted records enter the durable outbox and are persisted to the knowledge store before the endpoint reports success.
- `POST /aria/learning/persist` — retry durable outbox persistence after an interrupted hand-off; Reviewer or OrganizationAdmin role required.
- `POST /aria/memory` and `GET /aria/memory` — explicitly save and list only the authenticated user's encrypted personal memories; require the configured memory key file.
- `POST /aria/memory/{memory_id}/correct` and `DELETE /aria/memory/{memory_id}` — correct or forget only the authenticated user's memory; require the configured memory key file.

Owner and organization identifiers are derived from the authenticated Janus session, never from request fields. Text imports are private by default, and proposal review is an explicit cross-user organization action. Personal-memory endpoints are disabled unless the encrypted memory key file is configured. Every write is an explicit endpoint call; the server does not auto-save chat messages.

## Implemented unified library retrieval

`janus_core::aria::retrieval::search_all` combines results from:

- active knowledge records (including their stored provenance);
- source passages from preserved and access-scoped artifacts, with SHA-256 and byte locators;
- active personal memories belonging only to the owner ID supplied by the authenticated host.

Results are deterministically sorted by literal query match count. The host supplies independent knowledge and artifact scopes and the authenticated owner ID. The Janus Runtime exposes this through the authenticated `/aria/search` API. It is not yet wired into the scripted Aria chat panel, semantic/vector ranking, or model context assembly.

## Implemented read-only OmniVault snapshot import contract

`janus_core::aria::omnivault` accepts a normalized conversation snapshot supplied by a host after explicit user approval:

- The approval names the authenticated owner and the exact conversation IDs permitted for import.
- The adapter rejects conversations outside that allow-list, mismatched source IDs, future approval timestamps, empty conversations, and invalid source JSON.
- It copies the normalized source JSON snapshot unchanged into Aria's private, owner-scoped ingestion store and fingerprints it with SHA-256.
- The adapter exposes no OmniVault write, delete, or mutation operation. The original provider archive remains authoritative in OmniVault; this imported normalized snapshot is a separate read-only-derived copy.

This is a Rust import contract, not a live Android-to-Janus connector. The current OmniVault Android `VaultGateway` is in-process and does not expose a cross-app endpoint. A future authenticated host bridge must obtain consent, read only the selected conversations, and construct `OmniVaultReadApproval` from that authenticated grant. The normalized snapshot's hash is not represented as the original provider ZIP hash; do not confuse the two.

## Trust and memory boundaries

- Raw source artifacts remain authoritative; chunks and knowledge records are derived data.
- A model-generated proposal is untrusted until reviewed.
- Review acceptance means a human reviewed the proposal; it is not a claim that the source itself is authoritative or that the claim is universally true.
- Verification requires appropriate independent provenance and must not be inferred from model confidence.
- Hat labels classify knowledge; they do not grant tools or permissions.
- Personal conversation memory is a separate data class from shared technical knowledge. It must be scoped to an authenticated owner/user and must not be stored in organization-wide shared state.
- OmniVault imports must remain read-only. The host must obtain explicit user approval, preserve original archive identity and hashes, and never rewrite source archives.

## Remaining Stage 2 work

The following are not claimed as implemented:

- standards-complete PDF and DOCX parsers, full HTML5 parsing, image/OCR, and archive extraction adapters; the current HTML helper is deliberately conservative;
- an end-to-end authenticated host/API that connects artifact ingestion, proposal review, and knowledge persistence atomically;
- private per-user conversation events and explicit long-term memory controls;
- unified retrieval across artifact chunks, reviewed knowledge, and approved conversation context (artifact chunk search now returns source-cited passages);
- calibrated semantic/model-based Multicolored Hat classification with human correction; the deterministic keyword baseline is implemented but is only advisory;
- contradiction detection, re-review scheduling, retention/deletion workflows, and learning evaluation;
- Live authenticated OmniVault-to-Janus host bridge and user consent UI; the read-only snapshot import contract exists, but the current Android gateway has no cross-app endpoint;
- backup/restore coverage for all Aria-specific stores.

## Developer verification

Run from the repository root:

```sh
cargo fmt --all -- --check
cargo test -p janus-core aria::
cargo test --workspace
cargo check --workspace
```

A green CI run validates the code at that exact commit only. It does not prove the not-yet-built host integrations above.
