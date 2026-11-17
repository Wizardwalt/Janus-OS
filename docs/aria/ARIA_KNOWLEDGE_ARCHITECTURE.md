# Aria Knowledge Architecture — Multicolored Hat

**Status:** Initial architecture contract  
**Version:** 1.0.0

## Purpose

The Multicolored Hat is Aria's organizing model for a broad, connected knowledge base. It is not a set of disconnected chat personas. A request is routed to one or more knowledge domains, while shared evidence and provenance rules apply everywhere.

The machine-readable domain registry is `MULTICOLORED_HAT_REGISTRY.json`. The portable record contract is `KNOWLEDGE_RECORD_SCHEMA.json`.

## Logical components

1. **Intake** — accepts user-approved documents, imports, observations, and research results.
2. **Preservation adapter** — records original artifact identifiers and hashes. It never rewrites source originals.
3. **Normalizer** — extracts text and metadata into derived records while retaining source locators.
4. **Classifier** — proposes one or more hat/domain tags. Classification is revisable and does not imply verification.
5. **Evidence and provenance ledger** — tracks source, retrieval time, locator, epistemic status, corrections, and conflicts.
6. **Knowledge index** — supports keyword and semantic retrieval over approved records.
7. **Cross-domain orchestrator** — retrieves from multiple hats, compares evidence, and produces a synthesis that exposes important domain contributions.
8. **Policy and permission gateway** — authorizes tool use independently from knowledge retrieval.
9. **Evaluation loop** — tests retrieval quality, citation fidelity, contradiction handling, and correction behavior.

## Retrieval flow

1. Identify the user's task and constraints.
2. Select a primary hat and relevant secondary hats; include White Hat evidence handling for factual work.
3. Retrieve records using domain, tags, project scope, access policy, freshness, and relevance. The SQLite MVP now applies a conservative default scope (public/internal, unscoped records only); private/sensitive or project-scoped reads require an explicit scope supplied by the host after authorization.
4. Prefer primary and authoritative sources where appropriate; preserve disagreements.
5. Compose an answer that distinguishes sourced facts, inference, assumptions, and unknowns.
6. Attach source references for material claims.
7. Record user-approved corrections as new versions or superseding records, preserving history.

## Knowledge lifecycle

`ingest → preserve → extract → classify → review → index → retrieve → cite → correct/supersede`

No model output becomes verified merely because it sounds plausible or has been repeated. Automated classification and extraction are proposals until quality checks or human review establish the required status.

## Memory and knowledge separation

Personal memory is not the same as general knowledge. User-specific facts must have purpose, access controls, retention behavior, and deletion/correction pathways. Project knowledge must be scoped so Janus, OmniVault, and unrelated contexts do not leak into one another.

## Integration contracts

### OmniVault

OmniVault remains a standalone archive product. It can expose selected, permission-controlled conversation knowledge through a read-only adapter. Imported originals remain authoritative; Aria consumes derived text and source references, never silently edits or deletes originals.

### Janus OS

Janus can host Aria services and approved tools. The knowledge base does not itself authorize a module, shell command, device operation, network action, or hardware access. The host permission gateway must independently enforce capability grants and produce an audit record.

### Model providers

Provider adapters receive only the context needed for a task. Aria's stable knowledge record format, domain registry, and constitution remain provider-independent. Provider failure must not corrupt memory or knowledge.

## Initial implementation sequence

1. Establish and version the constitution, domain registry, and record schema. **This foundation is represented by this directory.**
2. Add schema validation and deterministic unit tests. **Rust record contracts and invariant checks are implemented in `janus-core::aria`.**
3. Implement a local durable knowledge store with provenance, versioning, and access-aware retrieval. **An initial SQLite-backed append-only store is implemented in `janus-core::aria::store`; it rejects ID overwrites, records supersession links, preserves old versions for audit, supports case-insensitive search and hat filtering, and enforces handling/project scopes on reads. The store does not authenticate users; the host remains responsible for constructing authorized scopes.**
4. Add scalable indexed retrieval and then semantic retrieval; the current MVP search scans active records and is intended for an initial local knowledge corpus.
5. Add source-grounded response assembly and contradiction reporting.
6. Add a provider-neutral model gateway.
7. Add read-only OmniVault integration and separately permissioned Janus tools.
8. Evaluate retrieval, privacy, reliability, and safety before expanding capabilities.

## Acceptance criteria for the foundation

- Every knowledge record can be classified under one or more registered hats.
- Every verified record has at least one source.
- Disputed records carry review notes.
- Corrections supersede rather than silently erase history.
- Domain classification is never treated as permission to execute tools.
- Original artifacts remain unchanged and addressable by stable IDs/hashes.
- Cross-domain answers retain provenance and disclose material uncertainty.


## Stage 2 — Memory and learning pipeline (initial slice)

The first Stage 2 implementation is a source-preserving UTF-8 text ingestion store in
`janus-core::aria::ingestion`.

- The host supplies a stable artifact ID, title, media type, source type, handling
  classification, project scope, import timestamp, and optional expiry.
- The exact original byte sequence is retained in SQLite and fingerprinted with
  SHA-256. Duplicate artifact IDs are rejected rather than overwritten.
- UTF-8 text is split into deterministic overlapping chunks. Each chunk carries
  its ordinal and byte offsets into the original artifact.
- Original bytes, manifests, and derived chunks are returned only when the
  caller-supplied host scope permits the artifact's handling and project labels.
  Expired artifacts are hidden from normal retrieval.
- Import and chunk creation occur in one SQLite transaction. A failed import
  cannot leave a partially indexed artifact.
- The current adapter accepts UTF-8 text only. It does not yet extract PDF,
  DOCX, HTML, images, audio, or video; those require dedicated, tested extractors.
- This store does not authenticate callers, classify content automatically,
  establish source truth, create personal memories, call an AI model, or train a
  model. The host must authenticate and authorize scopes before constructing them.

Next Stage 2 slices: document-format extraction with source locators; ingestion
job state and retry/idempotency policy; reviewed knowledge-record creation from
chunks; full-text indexing and source-grounded retrieval; user-controlled
personal memory lifecycle and deletion/correction; and evaluation for extraction
fidelity, citation accuracy, privacy boundaries, and retrieval quality.
