# ARIA Portability, Continuity, and Disaster Recovery

## Product requirement

ARIA must not be permanently tied to one Pip-Boy, one Monolith, one disk, or one operating-system installation. Her recoverable identity and learned state must be portable to another authorized Janus-OS host. A failed device must be replaceable without recreating ARIA or losing acknowledged memories.

This document is an architecture requirement, not a claim that full identity export, migration, replication, or failover is implemented in the current build.

## Current recovery foundation

- The runtime can create encrypted, consistent SQLite snapshots using SQLite's online backup API, including committed WAL state.
- Snapshots use XChaCha20-Poly1305 authenticated encryption and an owner-held 32-byte key supplied outside Janus.
- The runtime provides an offline `restore-backup` command. It authenticates the encrypted file, checks SQLite integrity, and creates a new database path without overwriting an existing destination.
- Temporary plaintext snapshot and restore files are created with owner-only permissions on Unix. On other platforms, their permissions inherit the containing directory's security ACL.
- A successful database restore is not yet a complete ARIA migration. Portable identity filtering, signed manifests, schema upgrade orchestration, artifact transfer, and device re-enrollment remain future work.

## Recovery key handling

Generate a fresh key using a cryptographically secure random generator, for example:

```sh
openssl rand -base64 32
```

Store the resulting value in a trusted secret manager or offline recovery record, separate from the Janus host and backup destination. Do not commit it, place it in a shell history, or store it beside the backup. Loss of the key makes the encrypted backup unrecoverable.

## Restore procedure

1. Install the same or a compatible Janus release on a trusted recovery host.
2. Place the encrypted `.janusbk` file on that host using a trusted transfer method.
3. Supply the recovery key through the process environment as `JANUS_BACKUP_KEY`.
4. Restore to a new path (the command refuses to overwrite existing files):

   ```sh
   JANUS_BACKUP_KEY='<base64-recovery-key>' janus-runtime restore-backup --backup ./backups/example.janusbk --destination ./recovery/janus-restored.db
   ```

5. Verify the restored database and retain the original encrypted backup unchanged.
6. Point the runtime at the restored database only after reviewing the recovery result.

The environment-variable example is convenient for automation but may expose the secret to process inspection on some systems. Prefer an OS secret manager or protected service environment for production operation. The command does not start the API server, load plugins, or initialize hardware.

## Portable-state boundary

A future signed, versioned ARIA bundle should include:

1. ARIA identity and user-approved personality/profile settings.
2. Long-term memory, conversation summaries, user corrections, and learning records.
3. Knowledge indexes and references to their source material.
4. Installed skill/module manifests, versions, dependency metadata, and integrity hashes.
5. User preferences, safe capability grants, and configuration.
6. Sync journal, replication checkpoints, schema version, and recovery metadata.
7. Optional model adapters and model configuration. Large model weights may be stored as separately verified content-addressed objects.

Hardware drivers, live device handles, network credentials, session tokens, private device keys, and machine-specific paths must not be copied as portable identity. On a new host, hardware is rediscovered and credentials are re-enrolled.

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

ARIA is considered portable only when a clean, separately provisioned Janus-OS host can restore her identity and all acknowledged learning state from a verified independent recovery bundle, continue operating without the original Pip-Boy or Monolith, and later reconcile safely with surviving replicas. A green build or a generated backup file alone does not satisfy this requirement.
