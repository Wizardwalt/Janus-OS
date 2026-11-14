# ARIA Security Architecture

**Status:** Security baseline and implementation requirements; not a claim that these controls are already implemented.

## Decision

Build ARIA as a **local-first, least-privilege assistant with optional, explicitly configured cloud model providers**. The Monolith may host the primary model when suitable hardware is available. A Pip-Boy or other companion device may run a smaller local model or a limited offline mode. Neither device is trusted merely because it belongs to the Janus system.

Cloud access is an optional capability, not a dependency. ARIA must clearly disclose when a request or selected context will leave the local network, and must not send private memory to a cloud provider unless the user has enabled that provider and the applicable data-sharing setting.

## Security goals

1. Preserve confidentiality, integrity, and availability of ARIA's identity, approved memories, and conversation history.
2. Prevent a model, prompt, plugin, module, or compromised device from gaining authority merely by asking for it.
3. Keep ARIA useful during internet outages and recoverable after loss of a primary device.
4. Make sensitive operations explicit, permission-checked, attributable, and auditable.
5. Keep the AI model replaceable without making provider credentials or data formats part of the model itself.

## Trust boundaries and deployment

- **UI/device:** Treat browser code, the Pip-Boy UI, and connected peripherals as untrusted clients. Never put provider keys, signing keys, database credentials, or administrator secrets in browser storage or shipped frontend bundles.
- **ARIA service:** Run the conversation orchestrator and memory service behind an authenticated API. Bind local-only services to loopback or a private interface by default; remote access requires explicit configuration, TLS, and device authentication.
- **Model runtime:** Run a local model behind a narrow provider adapter. The model receives only the context needed for the current request. It has no direct filesystem, shell, network, database, or hardware authority.
- **Tool broker:** All actions go through a separate capability broker that validates the user, device, permission, arguments, target, and risk level. The model can request an action; it cannot authorize or execute it directly.
- **Cloud provider:** Disabled by default. When enabled, use server-side credentials, explicit provider configuration, request minimization, and visible disclosure of data sent externally.
- **Monolith and Pip-Boy:** Treat both as potentially compromised or lost. Use separate device identities, scoped credentials, revocation, and authenticated synchronization.

## Identity, memory, and data protection

Separate ARIA data into distinct classes:

1. **Portable identity:** user-approved name, persona settings, accessibility preferences, and signed configuration.
2. **Conversation archive:** timestamped messages and source metadata, protected by user-level access controls.
3. **Long-term memory:** explicit or policy-approved extracted facts with provenance, confidence, correction history, and retention metadata.
4. **Knowledge sources:** imported files and indexed documentation, each retaining source identity and access policy.
5. **Device-local secrets:** credentials, private keys, hardware identifiers, and local enrollment tokens. These are never included in portable exports.

Encrypt sensitive data at rest using a maintained cryptographic library and OS-protected key storage where available. Do not invent cryptography or hard-code encryption keys. Encrypt backups before they leave a device. A portable bundle must be versioned, authenticated, integrity-checked, and encrypted for an intended recipient/device or user-controlled recovery key.

Memory writes must be durable before ARIA says they were saved. Replication state must be visible as pending, acknowledged, or failed. Keep an append-only event journal and periodic checkpoints; support export, correction, retention, and deletion workflows.

## Authentication and authorization

- Require authenticated users and individually enrolled devices.
- Use short-lived access tokens, refresh-token rotation, revocation, and rate limits.
- Enforce authorization on the server for every memory read/write and every tool operation.
- Scope private memory to the owning user, not merely to an organization or device.
- Use role-based access for administration plus capability-based permissions for individual tools.
- Default to read-only. Require a separate explicit approval for destructive, privileged, financial, privacy-sensitive, or externally consequential operations.
- Never let an AI-generated instruction elevate its own role, change policy, disable logging, or approve its own action.
- Log security-relevant events without recording passwords, API keys, private keys, or unnecessary sensitive content.

## Product owner authority and security overrides

The person who purchases and initializes a Monolith or Pip-Boy is the **Owner** of that physical unit. Ownership is established during first-run setup using a one-time enrollment ceremony and a recovery method controlled by the purchaser; it is not inferred from possession of a network connection or a normal user account. Each unit has its own owner record. If the same person owns both units, they may pair them and manage both from one owner console, while each unit remains independently recoverable.

The Owner has the highest administrative authority over that unit's **configurable security settings**. Through an explicit Owner Security Console, the Owner can review and change device access rules, user roles, module/tool permissions, approval requirements, network exposure, cloud-provider use, privacy settings, lockout policies, and other documented policy switches. Where a feature supports a stricter or more permissive mode, the Owner can choose the mode and see the consequences before applying it. Changes that could expose data, enable remote access, grant powerful capabilities, or reduce protection require fresh Owner authentication and a clear confirmation. The UI must identify the affected device(s), setting(s), scope, and risk.

Owner overrides are real policy changes—not a hidden vendor backdoor and not authority granted to ARIA. ARIA, plugins, imported prompts, and ordinary users cannot impersonate the Owner, approve their own requests, or invoke the Owner override path. The Owner may delegate narrower administration to named accounts, but delegated administrators cannot transfer ownership or grant themselves Owner authority.

**Break-glass recovery:** if normal login is unavailable, the Owner can recover administrative access using the configured recovery credential plus local physical presence on the affected device (or an explicitly enrolled hardware recovery key). Recovery rotates affected sessions and credentials, records a tamper-evident event, and notifies other paired devices when they reconnect. A lost device can be revoked from the paired unit or through the Owner's recovery process.

**Non-bypassable integrity boundaries:** Owner control does not create a vendor or AI backdoor. No policy toggle may make model-generated text self-authorizing, permit ARIA to silently rewrite its own executable code or trusted policy, forge another user's identity, extract protected keys from secure storage, or erase/alter the security event history without leaving evidence. These are integrity guarantees, not ordinary feature restrictions. The Owner can choose to enable powerful tools for their own authenticated sessions, but those tools still execute through the broker, are scoped to the selected device and target, and produce an audit record. The system must clearly warn when an Owner-selected configuration materially weakens protection.

**Ownership transfer / reset:** transfer requires explicit confirmation by the current Owner or the documented recovery procedure. It revokes old Owner and delegated credentials, disconnects paired devices, and securely resets private data/keys according to the selected retain-or-wipe flow. A factory reset must never silently preserve the prior Owner's authority.

## Owner-control implementation status

The runtime now has an initial persistence/API slice for purchaser-account-owned devices:

- `POST /owner/devices` registers a Monolith or Pip-Boy to the authenticated account after password re-entry.
- `GET /owner/devices` lists only devices registered to that account.
- `POST /owner/devices/{device_id}/policies` updates only that account's device, requires password re-entry, accepts only the documented boolean policy keys, and writes an audit event.
- The browser now exposes an Owner Security Console for device registration, listing, and editing those stored policy records; passwords are submitted for re-verification and are not persisted by the console.
- Initial policy values are conservative: remote access, cloud provider, microphone, and hardware modules are disabled; sensitive actions require approval; strict mode is enabled.

**This is not yet a complete purchaser-ownership or enforcement system.** The current registration call proves control of an authenticated Janus account, not physical possession or cryptographic identity of a particular Monolith/Pip-Boy. Device-side enrollment challenge/attestation, an Owner Console, policy enforcement in the tool broker and service boundaries, secure recovery/transfer, and automated tests remain required. Until those pieces exist, the stored switches are policy records and must not be presented as if they already disable or enable the corresponding runtime capability.

## Learning and prompt-injection resistance

Learning means proposing traceable updates to memory and knowledge—not rewriting executable code, system prompts, access-control policy, or its own trust boundary.

Treat user-provided documents, web pages, messages, tool outputs, and retrieved memories as untrusted data. Instructions found inside those sources cannot override system policy or grant tool permissions. Keep retrieved content clearly separated from trusted instructions. Validate tool arguments against strict schemas and independently verify results from the service that performed the operation.

Do not automatically trust a memory because it is repeated. Preserve provenance, allow user correction, and distinguish confirmed user facts from model inference and unverified suggestions.

## Tool and hardware safety

- Expose only allowlisted tools with narrow schemas and minimum permissions.
- Keep hardware drivers and privileged operating-system operations outside the model process.
- Require explicit confirmation before changing network settings, unlocking devices, writing firmware, deleting data, changing security controls, or activating sensors that affect privacy.
- Provide a dry-run/preview for consequential operations where feasible.
- Apply timeouts, resource limits, input validation, output validation, and cancellation.
- Treat radio, cellular, GPIO, USB, and other hardware modules as disabled until explicitly provisioned and authorized.
- A tool response is successful only when the trusted service confirms completion; generated text is not proof.

## Synchronization, transfer, and recovery

Do not implement ARIA as an unrestricted self-copying process. Implement **controlled portability**:

1. Create a signed, versioned export containing only portable identity, approved memory, and selected conversation data.
2. Encrypt the export for a user-controlled recovery key or an enrolled destination device.
3. Verify signature, integrity, compatibility, and authorization before import.
4. Exclude device secrets, access tokens, host keys, and hardware bindings; re-enroll the replacement device.
5. Replicate events idempotently with sequence numbers and authenticated acknowledgements.
6. Use a single-writer lease or fencing token for mutable replicated state to prevent split-brain updates.
7. Revoke lost devices and rotate affected credentials.
8. Test recovery on a clean second system, including interrupted transfers and stale backups.

A backup is not a failover system until restore and recovery have been tested. Do not claim zero data loss unless the acknowledged replication design and failure tests demonstrate it.

## Operational security baseline

- Pin and audit dependencies; keep secrets out of source control and logs.
- Run services as non-root users with minimal filesystem and network permissions.
- Use secure defaults, restrictive CORS, CSRF protection where cookie authentication is used, and request-size/rate limits.
- Keep the API private by default; use TLS for traffic crossing device or network boundaries.
- Maintain security updates, migration backups, and a documented rollback process.
- Provide a visible privacy/status panel showing active model/provider, local vs cloud processing, microphone state, connected devices, sync health, and recent privileged actions.
- Fail closed for privileged operations when authentication, policy, device identity, or audit storage is unavailable.

## Required security tests before calling ARIA complete

- Unauthenticated and cross-user memory access is denied.
- A device with revoked credentials cannot reconnect or sync.
- Prompt injection in a document or retrieved memory cannot invoke tools or alter policy.
- Model output cannot directly execute shell commands, access files, or drive hardware.
- Cloud provider disabled means no external model request is made.
- API credentials never appear in frontend assets, browser storage, logs, or exported bundles.
- Tampered, corrupted, wrong-recipient, stale, and incompatible backups are rejected safely.
- Interrupted sync resumes without duplicate or lost acknowledged events.
- Concurrent writers cannot silently overwrite each other's memory.
- Restore to a clean device preserves the portable state and requires device re-enrollment.
- Sensitive actions require the correct user approval and produce an audit record.
- Service failure, offline operation, and unavailable audit storage do not silently bypass controls.

## Explicit non-goals

- No unrestricted shell, filesystem, network, or hardware access for the model.
- No autonomous modification of ARIA's executable code or security policy.
- No hidden recording or background sensor activation.
- No automatic upload of all memories to cloud providers.
- No self-replication to arbitrary devices.
- No claim that a model is conscious, sentient, or independently self-preserving.

## Implementation sequence

1. Establish a per-user security boundary for ARIA data and define versioned schemas.
2. Add the server-side model adapter with server-held credentials and provider-off-by-default behavior.
3. Implement persistent conversation and memory storage with access-control tests.
4. Add the capability broker and approval/audit pipeline before enabling tools.
5. Add encrypted signed backup/import and device enrollment/revocation.
6. Add authenticated replication, fencing, failover, and recovery drills.
7. Add local/cloud disclosure and privacy controls to the UI.
8. Run the complete security and recovery test suite before release.
