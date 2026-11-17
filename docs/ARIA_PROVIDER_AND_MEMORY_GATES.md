# Aria Provider and Memory Delivery Gates

**Status:** Implementation contract. This document records required behavior and verification; it does not claim that live provider chat, user-scoped conversation persistence, or complete semantic retrieval is already connected.

## Non-negotiable boundaries

- The provider adapter runs server-side. Provider credentials never enter browser JavaScript, HTML, localStorage, or portable memory exports.
- Provider choice is configurable and replaceable. Do not couple Aria's identity, memory schema, or user-facing conversation model to one vendor.
- A local/offline fallback must be explicitly labeled as fallback behavior, never presented as a model response.
- Provider calls have bounded request/response sizes, deadlines, cancellation, and sanitized error reporting.
- Memory is user-scoped. Do not reuse organization-scoped generic state as a private memory namespace.
- A conversation answer must not claim to have remembered, learned, searched, or executed something unless the corresponding durable operation actually succeeded.
- Retrieval results retain source references and provenance; low-confidence or conflicting records must be presented as uncertain.

## Three implementation gates

### Gate 1 — Live provider adapter
Required work:
- Define a provider-neutral server-side request/response contract.
- Implement at least one configurable OpenAI-compatible endpoint and one local-model endpoint using the same contract.
- Keep secrets in protected runtime configuration, with redacted logs and owner-controlled rotation.
- Add timeout, cancellation, rate/size limits, provider health state, and deterministic offline fallback.

Acceptance:
- A configured provider answers through the Aria UI.
- Invalid credentials, provider outage, timeout, cancellation, malformed response, and oversized response are tested.
- Browser assets and browser storage contain no provider secret.
- UI labels fallback output and provider state honestly.

### Gate 2 — Durable, private conversation history
Required work:
- Persist conversation events under an authenticated user identity.
- Enforce ownership in every read, append, edit, export, and delete path.
- Add migrations, retention configuration, export, deletion, and audit coverage.
- Keep conversation history separate from extracted long-term memory.

Acceptance:
- Conversation survives process restart.
- Cross-user read/write attempts are denied.
- Export and deletion tests verify the actual persisted records.
- Failed persistence is surfaced as failure, not acknowledged as saved.

### Gate 3 — Evidence-linked memory retrieval
Required work:
- Retrieve only records authorized for the current user and task.
- Return source/event identifiers, timestamps, and confidence/provenance metadata.
- Support correction, review, and explicit deletion of remembered facts.
- Keep retrieval read-only with respect to executable code, capabilities, and security policy.

Acceptance:
- Tests prove user isolation, source traceability, correction precedence, deletion behavior, and deterministic handling of conflicting facts.
- The UI can show which memory records informed an answer.
- No automatic memory extraction is enabled without an explicit approved policy.

## Release rule

A gate is complete only after implementation, automated tests, and runtime evidence are available. Documentation, a UI mock, a scripted response, or a passing compile alone does not satisfy a gate.
