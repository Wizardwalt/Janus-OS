# Aria Authenticated Conversation API

**Status:** Authenticated API and encrypted storage integration. The browser chat UI and a live model provider are not connected by this change.

## Storage and key configuration

The runtime opens the encrypted conversation store only when an Aria data key is configured. Set:

```sh
JANUS_ARIA_DATA_KEY_FILE=/absolute/path/to/aria-data.key
```

The file must contain exactly 64 hexadecimal characters (32 bytes). On Unix-like systems, it must be a regular, non-symlink file with owner-only permissions (`0600`). The legacy variable `JANUS_ARIA_MEMORY_KEY_FILE` remains accepted for compatibility.

The key is supplied by the host and is never stored in SQLite. Aria derives a separate conversation-encryption key from it using SHA-256 domain separation. Conversation event payloads are encrypted with XChaCha20-Poly1305. Owner ID, conversation ID, event ID, and timestamp are authenticated as associated data. Those identifiers and timestamps remain queryable metadata; message content, role, provider, and model are encrypted.

Without the key, the runtime does not silently fall back to plaintext conversation storage. Conversation endpoints return HTTP 503. Existing plaintext rows are migrated transactionally when the key is configured; a failed migration leaves the old table intact.

## Authentication and ownership

Every endpoint below requires the existing Janus bearer session:

```http
Authorization: Bearer <session-token>
```

The runtime derives the owner ID from the authenticated account. No endpoint accepts an owner ID from the request body or query string.

## Endpoints

- `GET /aria/conversations?limit=20` — list the authenticated user's conversations (maximum 100).
- `GET /aria/conversations/{conversation_id}/events?limit=100` — read that owner's events (maximum 1,000).
- `POST /aria/conversations/{conversation_id}/events` — append an event. The conversation is created on first append.
- `DELETE /aria/conversations/{conversation_id}` — delete that owner's conversation and cascade-delete its events.

Append request example:

```json
{
  "event_id": "client-generated-stable-id",
  "role": "user",
  "content": "Hello, Aria",
  "provider": "local",
  "model": "configured-model"
}
```

`created_at` is optional and defaults to the runtime's current UTC time. Roles are restricted by the storage layer to `system`, `user`, `assistant`, or `tool`. Event IDs are unique within an owner namespace, not globally.

## Remaining integration gates

1. Add authenticated browser/UI calls for list, append, load, and delete.
2. Connect a provider-neutral server-side model adapter; keep provider credentials out of browser assets and local storage.
3. Persist the user message before provider dispatch and persist the assistant response before acknowledging completion.
4. Add end-to-end API tests for missing/expired sessions, cross-owner access, unavailable key configuration, append/read/delete, and interrupted provider calls.
5. Add owner-facing retention, export, and deletion controls before production release.

A passing compile or storage test does not complete these UI, provider, or operational gates.
