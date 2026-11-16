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
7. The host is responsible for authorizing the reviewer and persisting an accepted record into `KnowledgeStore`.

The queue is a library component, not an authenticated API. Hosts must enforce user identity, artifact access, reviewer authorization, and project/handling scopes before calling it. The proposal queue and knowledge store are separate SQLite components; the host must handle the hand-off and report success only after both the review decision and accepted record are durably committed. A future host-level transaction coordinator should make that hand-off atomic.


## Implemented explicit personal-memory store

`janus_core::aria::memory` provides a separate SQLite store for explicit personal memories:

- Each memory has an owner ID; reads, corrections, lists, and forget operations require the matching owner ID.
- New memories must be explicitly written by the host. The store does not automatically turn conversation messages into memories.
- Corrections increment a version and retain an audit event containing the prior content digest.
- Forgetting deletes the stored content and source references and clears prior content digests from the audit trail, retaining only minimal event metadata.
- Expired memories are hidden from reads and lists and cannot be corrected.
- Unit tests cover owner isolation, correction history, forget behavior, and expiry.

The owner ID is an authorization input supplied by the host, not proof of identity. The host must authenticate the user before calling the store. This library component is not yet connected to the ARIA chat API or user interface. SQLite contents are not encrypted by this component; production hosts must provide OS/database encryption and protected key management before storing sensitive personal memories.

## Implemented unified library retrieval

`janus_core::aria::retrieval::search_all` combines results from:

- active knowledge records (including their stored provenance);
- source passages from preserved and access-scoped artifacts, with SHA-256 and byte locators;
- active personal memories belonging only to the owner ID supplied by the authenticated host.

Results are deterministically sorted by literal query match count. The host supplies independent knowledge and artifact scopes and the authenticated owner ID. This is a library-level retrieval API; it is not yet wired into the Aria chat UI, semantic/vector ranking, or a production identity/session service.

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
