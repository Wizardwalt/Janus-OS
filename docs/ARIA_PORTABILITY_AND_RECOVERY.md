# ARIA Portability, Continuity, and Disaster Recovery

## Product requirement

ARIA must not be permanently tied to one Pip-Boy, one Monolith, one disk, or one operating-system installation. Her recoverable identity and learned state must be portable to another authorized Janus-OS host. A failed device must be replaceable without recreating ARIA or losing acknowledged memories.

This document describes the full target architecture. The current implementation includes encrypted whole-database backup and a verified restore command; portable identity/memory export, independent replica orchestration, failover fencing, and Android migration are still incomplete.

## Operating model: local-first, replicated intelligence

- **Pip-Boy runtime:** keeps a local, usable ARIA instance, recent conversation context, device observations, and a durable local change journal. It must continue in a clearly indicated offline/degraded mode when the Monolith is unreachable.
- **Monolith:** provides the primary shared model-serving and coordination service when available, plus a durable replica of ARIA's portable state.
- **Recovery host:** a third Janus-OS computer or encrypted removable backup can restore ARIA if either primary system is lost. The recovery host must be independently bootable and must not require the failed host to authorize recovery.
- **Portable identity:** ARIA's stable identity is a signed, versioned data bundle, not a hardware serial number, hostname, disk, or model installation. Hardware-specific bindings are stored separately and re-detected on the replacement host.

## What must be portable

The portable state bundle should include:

1. ARIA identity and user-approved personality/profile settings.
2. Long-term memory, conversation summaries, user corrections, and learning records.
3. Knowledge indexes and references to their source material.
4. Installed skill/module manifests, versions, dependency metadata, and integrity hashes.
5. User preferences, safe capability grants, and configuration.
6. Sync journal, replication checkpoints, schema version, and recovery metadata.
7. Optional model adapters and model configuration. Large model weights may be stored as separately verified content-addressed objects.

Hardware drivers, live device handles, network credentials, session tokens, and machine-specific paths must not be copied as if they were portable identity. On a new host, hardware is rediscovered and credentials are re-enrolled.

## Durability and replication rules

- Record every accepted memory/configuration change in a durable append-only journal before acknowledging it to the user.
- Replicate journal entries to the Monolith and at least one independent recovery target. Display the last successful replication time and any unreplicated changes.
- Use a transactional, consistent database snapshot (including SQLite WAL state where applicable); never copy a live database file naively.
- Store large artifacts by content hash, verify hashes on transfer, and retain versioned manifests so interrupted transfers can resume.
- Use encrypted transport and encryption at rest. Recovery keys must be held separately from the devices being backed up; losing both a device and its only decryption key must not be treated as recoverable.
- Keep at least three copies on two different storage types, with one encrypted copy physically or logically separate from both primary systems.
- Preserve historical snapshots and journal checkpoints so accidental deletion, corruption, or a bad update can be rolled back.
- Do not report a change as fully protected until the required replica acknowledgements are received. If only the local journal has it, label it **pending replication**.

## Failover and split-brain protection

- If the Monolith disappears, the Pip-Boy continues from its local durable state and queues changes for later synchronization.
- If the Pip-Boy disappears, the Monolith continues serving ARIA and can provision a replacement device.
- If both are unavailable, restore from the independent recovery copy.
- Each active writer must hold a time-bounded, renewable lease with a monotonically increasing fencing epoch. A stale host must not overwrite newer state when it reconnects.
- Merge independent append-only learning events by stable event ID and causal metadata. For conflicting profile/configuration edits, preserve both versions and require an explicit resolution policy rather than silently choosing one.
- Failover must never silently discard queued local events. Synchronization should be resumable and idempotent.

## Current implemented recovery slice

The runtime can create encrypted local SQLite snapshots when configured with an externally held 32-byte key. Restore is available as a one-shot command:

```sh
JANUS_BACKUP_KEY='<base64-encoded-32-byte-recovery-key>' \
  janus-runtime --restore-backup ./backups/example.janusbk \
  --restore-db-path ./recovered/janus.db
```

The destination must not already exist. Restore authenticates/decrypts the backup, writes to a staging file, clears old login sessions, pending enrollment challenges, and enrolled device public-key bindings, runs SQLite integrity verification, then installs the database without overwriting an existing path. The recovered host must re-enroll its Monolith/Pip-Boy devices and re-detect hardware. Keep the original backup unchanged.

This is a **full database disaster-recovery snapshot**, not yet a selective portable ARIA identity bundle. It can contain account records and organization data; protect the recovery key and restored database accordingly. A later portability phase must export only approved ARIA identity, memory, preferences, learning and sync data while excluding password hashes, sessions, host secrets and device bindings.

## Restore and migration workflow

1. Boot a clean, trusted Janus-OS installation on the replacement host.
2. Authenticate the owner and unlock the encrypted ARIA recovery bundle using a separately held recovery key.
3. Verify the bundle signature, manifest, hashes, schema compatibility, and snapshot/journal chain before importing anything.
4. Restore ARIA identity, memories, learning journal, preferences, and approved skill manifests into a staging area.
5. Re-detect hardware and show unavailable capabilities; never assume the new device has the old device's sensors or permissions.
6. Re-enroll device credentials and issue new host-specific secrets. Never restore old session tokens or private device keys.
7. Run consistency checks and a read-only startup test. Activate the restored instance only after verification.
8. Reconnect replicas, reconcile journal events, and confirm the replication watermark.
9. Retain the source backup unchanged until the owner confirms successful recovery.

## Security and owner control

"Unlocked AI" means broad learning, reasoning, customization, and portability—not unrestricted authority over every host. ARIA may inspect available hardware through explicitly installed adapters and may prepare code or configuration changes. Privileged, destructive, privacy-sensitive, or externally consequential actions require scoped authorization, an audit record, and a recoverable rollback path. Updates to ARIA herself must be staged, tested, signed, and reversible.

## Required recovery tests

- Restore a complete ARIA identity and memory bundle onto a clean second machine.
- Lose the Pip-Boy while the Monolith remains available, then provision a replacement.
- Lose the Monolith while the Pip-Boy remains available, then rebuild the Monolith from the Pip-Boy plus independent backup.
- Disconnect both primary systems and restore from the independent recovery copy.
- Interrupt replication mid-transfer and prove it resumes without duplicate or missing events.
- Reconnect a stale device and prove fencing prevents it from overwriting newer state.
- Corrupt an artifact and prove hash verification detects it before activation.
- Verify that secrets and old host sessions are not copied into the portable bundle.

## Acceptance criteria

ARIA is considered portable only when a clean, separately provisioned Janus-OS host can restore her identity and all acknowledged learning state from a verified recovery bundle, continue operating without the original Pip-Boy or Monolith, and later reconcile safely with surviving replicas. A green build or a generated backup file alone does not satisfy this requirement.
