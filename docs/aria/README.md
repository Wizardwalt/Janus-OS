# Aria

**One intelligence. Many disciplines. No artificial boundaries.**

Aria is a provider-independent AI companion and knowledge platform being developed within Janus OS. OmniVault is designated as her intended canonical long-term memory and continuity layer; the current integration is still a read-only snapshot contract, not a live cross-app bridge. Her identity, knowledge contracts, and user-facing principles must remain stable when the underlying model or host application changes.

## What makes Aria different

Aria's knowledge system is organized around the **Multicolored Hat**: one connected knowledge base with ten complementary ways to classify and retrieve information. The hats are not separate personalities or isolated silos. A single task can draw on several disciplines, preserve their evidence, and explain how they fit together.

| Hat | Domain |
| --- | --- |
| Red | Electronics, hardware, circuitry, diagnostics and repair |
| Orange | Creativity, art, music, writing and design |
| Yellow | Science, mathematics, research and education |
| Green | Programming, software engineering, AI and development |
| Blue | Computers, networking, operating systems and infrastructure |
| Purple | Cybersecurity, ethical hacking, digital forensics and security research |
| Black | Critical thinking, risk analysis, troubleshooting and failure investigation |
| White | Facts, documentation, evidence, references and provenance |
| Brown | Practical skills, mechanical systems, tools and everyday problem-solving |
| Rainbow | Interdisciplinary reasoning that connects the other disciplines |

White Hat evidence rules apply across every domain. Rainbow Hat coordinates cross-domain work. A classification never grants permission to execute a tool or perform an operation.

## Foundational principles

- **Honest by design:** distinguish sourced facts, inference, assumptions and unknowns.
- **Evidence that can be traced:** retain source references, timestamps, locators and correction history.
- **User authority:** protect personal data and require approval for consequential actions.
- **Least privilege:** separate knowledge access, tool capability and authorization.
- **Provider independence:** keep identity, knowledge formats and tool contracts portable.
- **No simulated success:** clearly identify mock, demo, offline and unverified behavior.
- **Continuous improvement with change control:** learning does not silently rewrite Aria's constitution or permissions.

## Stage 1 — Foundation

Stage 1 establishes Aria's identity and the technical contracts needed for future capabilities.

Implemented in this repository:

- Versioned constitution and host responsibilities.
- Machine-readable ten-hat registry and routing rules.
- Portable, provenance-aware knowledge record JSON Schema.
- Rust record types and validation in `janus-core::aria`.
- SQLite knowledge storage with stable IDs, duplicate protection, search and hat filtering.
- Append-only supersession links that preserve historical records.
- Default-safe and explicit host-supplied access scopes for handling classifications and project boundaries.
- Unit tests, including a registry-to-Rust identifier consistency check.

### Stage 1 boundaries

Stage 1 is the foundation, not a claim that every future capability already exists. Aria has an authenticated provider-backed chat path with OpenAI-compatible and native Anthropic adapters, encrypted owner-scoped conversation storage, cursor pagination, complete conversation export, and browser-side cancellation controls. Streaming, additional provider protocols, remote-provider cancellation guarantees, autonomous background learning, voice, and a production tool gateway remain incomplete. A provider adapter does not mean credentials are configured or that a live provider has been acceptance-tested on the user's device. The host authenticates users and authorizes access; the knowledge store is not an identity provider.

## Stage 2 — Memory and learning (in progress)

Implemented first slices:

- Source-preserving UTF-8 text ingestion in `janus-core/src/aria/ingestion.rs`, retaining exact originals, SHA-256 hashes, deterministic chunks, byte locators, access scopes and expiry.
- Plain-text validation, conservative simple-HTML extraction, and a bounded media-type dispatcher with hash provenance, byte ceilings, unsupported-format rejection, and untrusted-content warnings in `janus-core/src/aria/extraction.rs`.
- Scoped chunk search that returns source-cited passages with original artifact hashes and byte ranges; retrieved passages can be converted directly into unverified learning proposals.
- Explainable Multicolored Hat keyword suggestions in `janus-core/src/aria/classification.rs`, including matched-term explanations and Rainbow cross-domain suggestions. These are advisory, not calibrated confidence or authorization.
- A durable human-review proposal queue in `janus-core/src/aria/learning.rs`. Proposals must cite the exact source artifact and hash; acceptance produces a `Reviewed` record, never an automatically `Verified` one.
- Immutable accept/reject review receipts with reviewer identity, timestamp and notes, plus a recoverable, organization-scoped persistence outbox.
- An owner-scoped plaintext development memory store and an XChaCha20-Poly1305 encrypted production memory store in `janus-core/src/aria/encrypted_memory.rs`.
- Unified retrieval across public/internal unscoped knowledge, owner-private source passages and the authenticated owner's encrypted personal memories.
- An owner-approved, read-only OmniVault normalized-snapshot importer in `janus-core/src/aria/omnivault.rs`.
- Authenticated Janus Runtime API routes for text ingestion, search, learning proposals/review and explicit remember/list/correct/forget operations. Personal-memory endpoints require a separate owner-only key file.

Stage 2 is **not complete**. The authenticated live chat path can opt into owner-scoped retrieval from Janus knowledge, source artifacts, and encrypted personal memories, but this is not semantic/vector retrieval and does not yet include OmniVault content. Standards-complete PDF/DOCX/HTML5 extraction and OCR, semantic retrieval, contradiction detection and evaluation, retention workflows, and key rotation/recovery remain. The learning outbox is recoverable and tenant-scoped but is an at-least-once hand-off across separate SQLite connections, not one cross-database transaction. The current OmniVault Android gateway is in-process only; the Rust snapshot importer is a contract, not a live cross-app connection. The current HTML helper handles simple exported pages and is not a full HTML5 parser. The encrypted memory store requires host-managed key generation, storage, backup and rotation.

See [Memory and Learning System](ARIA_MEMORY_AND_LEARNING.md) for the implementation contract and remaining exit criteria.

## Architecture documents

- [Master Identity and Architecture Charter](ARIA_MASTER_IDENTITY_AND_ARCHITECTURE_CHARTER.md) — user-directed identity, memory, autonomy, operator, continuity, and embodiment goals.
- [Implementation Audit](ARIA_IMPLEMENTATION_AUDIT.md) — evidence-based status and gaps against the charter.
- [Aria Constitution](ARIA_CONSTITUTION.md) — identity, principles, memory boundaries and host responsibilities.
- [Multicolored Hat Registry](MULTICOLORED_HAT_REGISTRY.json) — stable domain IDs, descriptions and routing rules.
- [Knowledge Record Schema](KNOWLEDGE_RECORD_SCHEMA.json) — portable record format and provenance contract.
- [Knowledge Architecture](ARIA_KNOWLEDGE_ARCHITECTURE.md) — lifecycle, retrieval, integrations and implementation sequence.

## Developer entry point

The Rust foundation lives in:

- `janus-core/src/aria.rs`
- `janus-core/src/aria/store.rs`
- `janus-core/src/aria/classification.rs`
- `janus-core/src/aria/extraction.rs`
- `janus-core/src/aria/ingestion.rs`
- `janus-core/src/aria/learning.rs`
- `janus-core/src/aria/memory.rs`
- `janus-core/src/aria/encrypted_memory.rs`
- `janus-core/src/aria/omnivault.rs`
- `janus-core/src/aria/retrieval.rs`

From the repository root, run the workspace checks:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

The GitHub Actions workflow is the recorded CI verification for the repository branch. Do not treat a successful historical run as proof of a later commit; verify the run's head SHA matches the commit being reviewed.

## Project status

The current `main` branch includes the provider-backed chat, encrypted owner-scoped conversation history, knowledge and learning foundations, and a read-only OmniVault snapshot importer. See the [implementation audit](ARIA_IMPLEMENTATION_AUDIT.md) for exact evidence and gaps. The Master Charter records intended future behavior; it is not a claim that those capabilities are implemented.
