# Aria Provider and Memory Delivery Gates

**Status:** Implementation contract. Authenticated provider-backed chat and encrypted user-scoped conversation storage are connected. This document records the remaining behavior and verification gates; it does not claim streaming, complete provider coverage, export/retention, or operational key recovery are finished.

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
- Keep the provider-neutral contract and support the existing OpenAI-compatible protocol plus the native Anthropic Messages protocol. Local model servers may use the OpenAI-compatible adapter when they implement that protocol.
- Keep secrets in protected runtime configuration, with redacted logs and owner-controlled rotation.
- Preserve bounded request/response sizes, timeouts, cancellation, provider health state, and deterministic offline fallback; add API-level failure-injection tests and streaming.

Acceptance:
- A configured provider answers through the Aria UI.
- Invalid credentials, provider outage, timeout, cancellation, malformed response, and oversized response are tested.
- Browser assets and browser storage contain no provider secret.
- UI labels fallback output and provider state honestly.

### Gate 2 — Durable, private conversation history
Required work:
- Persist conversation events under an authenticated user identity; owner-scoped list/read/append/delete and cursor pagination are implemented.
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
- Preserve knowledge IDs, epistemic status, source URIs, source artifact IDs/chunk ordinals/hashes, and personal-memory IDs/source references in model context.
- Support correction, review, and explicit deletion of remembered facts.
- Keep retrieval read-only with respect to executable code, capabilities, and security policy.

Acceptance:
- Tests prove user isolation, source traceability, correction precedence, deletion behavior, and deterministic handling of conflicting facts.
- The UI can show which memory records informed an answer.
- No automatic memory extraction is enabled without an explicit approved policy.

## Release rule

A gate is complete only after implementation, automated tests, and runtime evidence are available. Documentation, a UI mock, a scripted response, or a passing compile alone does not satisfy a gate.
