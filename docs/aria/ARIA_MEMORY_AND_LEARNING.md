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

- PDF, DOCX, HTML, image/OCR, and archive extraction adapters;
- an end-to-end authenticated host/API that connects artifact ingestion, proposal review, and knowledge persistence atomically;
- private per-user conversation events and explicit long-term memory controls;
- retrieval that combines artifact chunks, reviewed knowledge, and approved conversation context with citations;
- automatic Multicolored Hat suggestions with confidence and human correction;
- contradiction detection, re-review scheduling, retention/deletion workflows, and learning evaluation;
- OmniVault read-only connector and user consent UI;
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
