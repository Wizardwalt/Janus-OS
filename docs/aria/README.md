# Aria

**One intelligence. Many disciplines. No artificial boundaries.**

Aria is a provider-independent AI assistant and knowledge platform being developed within Janus OS, with a planned read-only knowledge integration for OmniVault. Her identity, knowledge contracts, and user-facing principles are designed to remain stable even when the underlying AI model or host application changes.

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

Stage 1 is the foundation, not a claim that every future capability already exists. Aria does not yet have a connected live model provider, autonomous learning pipeline, document-ingestion service, semantic/vector retrieval, voice interface, or production tool gateway. The host application must authenticate users and authorize any broader access scope; the knowledge store enforces a scope but is not an identity provider.

## Architecture documents

- [Aria Constitution](ARIA_CONSTITUTION.md) — identity, principles, memory boundaries and host responsibilities.
- [Multicolored Hat Registry](MULTICOLORED_HAT_REGISTRY.json) — stable domain IDs, descriptions and routing rules.
- [Knowledge Record Schema](KNOWLEDGE_RECORD_SCHEMA.json) — portable record format and provenance contract.
- [Knowledge Architecture](ARIA_KNOWLEDGE_ARCHITECTURE.md) — lifecycle, retrieval, integrations and implementation sequence.

## Developer entry point

The Rust foundation lives in:

- `janus-core/src/aria.rs`
- `janus-core/src/aria/store.rs`

From the repository root, run the workspace checks:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

The GitHub Actions workflow is the recorded CI verification for the repository branch. Do not treat a successful historical run as proof of a later commit; verify the run's head SHA matches the commit being reviewed.

## Project status

Aria's foundation is under active development on the `aria/multicolored-hat-foundation` branch and tracked in [Pull Request #10](https://github.com/Wizardwalt/Janus-OS/pull/10). It remains a draft until the owner chooses to approve its review and integration.
