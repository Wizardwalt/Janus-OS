# Aria Ecosystem Integration Contract

**Status:** Stage 1 architecture contract  
**Version:** 1.0.0  
**Applies to:** Aria, Janus OS, OmniVault, Janus Community, and the Multicolored Hat Society

## 1. Identity and independence

Aria is the shared, provider-independent assistant identity and coordination layer. Her constitution, evidence rules, memory boundaries, and user-facing behavior must remain stable when the underlying model provider or host application changes.

Aria does not absorb the products she connects:

- **Janus OS** owns device, electronics, operating-system, and authorized technical operations.
- **OmniVault** owns preservation and organization of original conversation archives and the user's local knowledge vault.
- **Janus Community** owns its own public technology community, accounts, moderation, data, and website.
- **Multicolored Hat Society** owns its own cybersecurity education and research community, accounts, moderation, data, and website.

Each product must remain useful if Aria, a model provider, another product, or the network is unavailable.

## 2. Two different uses of “Multicolored Hat”

These must not be conflated.

### Aria's internal knowledge taxonomy — ten hats

The ten-hat registry in `MULTICOLORED_HAT_REGISTRY.json` organizes Aria's connected knowledge base:

- Red — electronics and hardware
- Orange — creativity and expression
- Yellow — science, mathematics, and learning
- Green — software, programming, and AI
- Blue — computing and infrastructure
- Purple — cybersecurity, digital forensics, and security research
- Black — critical thinking, risk analysis, and failure investigation
- White — evidence, documentation, facts, and provenance
- Brown — practical skills, mechanical systems, and everyday problem-solving
- Rainbow — interdisciplinary synthesis

These are retrieval/classification lenses, not separate accounts, permission levels, or executable roles. White Hat evidence rules apply across all domains; Rainbow coordinates cross-domain work.

### The independent Multicolored Hat Society — six communities

The Society is a separate public learning and collaboration platform with six community perspectives:

- White Hat
- Black Hat
- Gray Hat
- Blue Hat
- Red Hat
- Green Hat

The Society's six community identities are not the same thing as Aria's ten internal knowledge domains. The Society's Black Hat track may study malicious techniques to understand threats and improve defense, but must not operationalize them to harm people or compromise third-party systems. Practical activity requires owned systems, explicit authorization, or isolated labs; responsible disclosure and defensive learning are supported.

No Society membership or hat identity grants permission to execute Janus tools or access OmniVault data.

## 3. Product integration boundaries

### Aria ↔ Janus OS

- Janus OS authenticates the user and owns authorization for every system/device action.
- Aria may request an operation through a documented, versioned API.
- Janus validates role, engagement, target asset, certification, and requested capabilities independently.
- A knowledge record, model output, or Society hat label never authorizes execution.
- Every consequential operation is previewed, explicitly approved, scoped, and audited.
- If authorization or audit persistence fails, execution fails closed.

### Aria ↔ OmniVault

- OmniVault remains local-first and provider-independent.
- Original provider exports remain byte-for-byte authoritative and are never rewritten by Aria.
- Any Aria import is a derived, source-linked snapshot, not a replacement for the original.
- Access is opt-in and owner-scoped to selected conversations or artifacts.
- The first integration contract is read-only; write, delete, or vault-wide access is not implied.
- OmniVault's AI Gateway must enforce explicit grants, scope limits, revocation, and audit before production use.
- Aria must remain functional without OmniVault, and OmniVault must remain useful without Aria.

The current Rust `janus_core::aria::omnivault` module is a read-only import contract that accepts an owner-approved normalized snapshot. It is not evidence of a live cross-app connection. The Android gateway is currently described as in-process; a production cross-app API remains future work.

### Aria ↔ Communities

- Aria may provide consistent attribution and optional assistant features where actually connected.
- Janus Community and the Multicolored Hat Society maintain separate repositories, websites, databases, moderation, terms, privacy policies, and release processes.
- Do not share accounts or databases by default. If shared identity is later selected, it must be an explicitly designed identity service with separate product authorization and revocation.
- Community submissions are untrusted until reviewed; provenance and status (owner-tested, independently verified, experimental, failed, deprecated) must be visible.
- Do not imply Aria is online or providing a service when the integration is offline or not yet implemented.

## 4. Permission vocabulary

Every integration must distinguish:

- **Discover:** identify that a resource exists.
- **Read:** retrieve only explicitly selected data.
- **Propose:** suggest a change without applying it.
- **Write:** change data only after a separately granted scope and approval.
- **Execute:** perform an operation through the owning product's authorization boundary.
- **Administer:** manage users, policies, keys, or product configuration; never implied by ordinary read/write access.

Permissions are scoped, time-bounded where practical, revocable, and logged. A model provider receives only the context needed for the current user-approved task.

## 5. Data and provenance rules

- Preserve source identity, source hash where available, source locator, timestamps, and epistemic status.
- Derived knowledge must link to its original source.
- Corrections supersede prior knowledge with an audit trail; they do not silently rewrite history.
- Personal/private records require authenticated owner context.
- Secrets and encryption keys are never committed to source control or stored beside the protected data they unlock.
- Data export, deletion, retention, and recovery must be explicit and testable.

## 6. Provider independence and capability honesty

- Model providers are replaceable inference/generation backends, not owners of Aria's identity or memory.
- The host exposes tool interfaces through a provider-neutral contract.
- If a model or external service is unavailable, products degrade independently and clearly.
- Every feature is labelled implemented-and-tested, implemented-but-unverified, simulated/demo, stub/placeholder, or planned.
- No scripted avatar text, canned response, or demo output is presented as live model reasoning, telemetry, or a completed device operation.

## 7. Stage 1 implementation status

Already present in the current Janus-OS Aria foundation branch:
- Constitution and ten-domain registry.
- Portable provenance-aware knowledge record schema.
- Rust knowledge contracts and SQLite store.
- Source-preserving ingestion, bounded extraction, scoped retrieval, and human-review learning queue.
- Owner-scoped memory interfaces and encrypted-memory implementation requiring host-managed keys.
- Authenticated runtime routes for explicit memory and knowledge operations.
- A read-only, owner-approved OmniVault normalized-snapshot import contract.

Still not complete:
- A live model provider and chat-context assembly.
- Full semantic/vector retrieval and quality evaluation.
- Production key generation, backup, rotation, and recovery workflows.
- A live cross-app OmniVault AI Gateway; the Rust snapshot importer is a contract, not an operational bridge.
- Separate repositories and deployed websites for Janus Community and the Multicolored Hat Society.
- Production identity federation, moderation, and account integrations across the community platforms.

## 8. Change control

This contract supplements, and does not silently replace, Aria's constitution or either product's charter. Changes to user authority, privacy, permissions, product ownership, or the six Society tracks require a reviewed, versioned change. Any production deployment, external-provider connection, domain/hosting purchase, or destructive migration requires separate explicit approval.
