# Aria Authenticated Conversation API

**Status:** Encrypted conversation storage, authenticated runtime endpoints, OpenAI-compatible and native Anthropic provider adapters, and browser chat/history controls are implemented in this branch. Live inference requires a configured provider and protected data key.

## Encrypted storage and key configuration

Set:

```sh
JANUS_ARIA_DATA_KEY_FILE=/absolute/path/to/aria-data.key
```

The file must contain exactly 64 hexadecimal characters (32 bytes). On Unix-like systems, it must be a regular, non-symlink file with owner-only permissions (`0600`). The legacy variable `JANUS_ARIA_MEMORY_KEY_FILE` remains accepted for compatibility.

The key is supplied by the host and is never stored in SQLite. Aria derives a separate conversation-encryption key from it using SHA-256 domain separation. Conversation event payloads are encrypted with XChaCha20-Poly1305. Owner ID, conversation ID, event ID, and timestamp are authenticated as associated data. Those identifiers and timestamps remain queryable metadata; message content, role, provider, and model are encrypted.

Without the key, the runtime does not silently fall back to plaintext conversation storage. Conversation endpoints return HTTP 503. Existing plaintext rows are migrated transactionally when the key is configured; a failed migration leaves the old table intact.

## Model provider configuration

The runtime supports an OpenAI-compatible Chat Completions endpoint by default. Configure both:

```sh
JANUS_ARIA_PROVIDER_CHAT_COMPLETIONS_URL=https://provider.example/v1/chat/completions
JANUS_ARIA_PROVIDER_MODEL=your-model-id
JANUS_ARIA_PROVIDER_NAME=provider-label
JANUS_ARIA_PROVIDER_API_KEY_FILE=/absolute/path/to/provider-api-key
```

The provider name and API-key file are optional; endpoint and model must be configured together. The API-key file, when used, must be a regular non-symlink file, 1..=4096 bytes, owned by the Janus runtime user, and owner-only (0600) on Unix. Keep it on the runtime host; it is never sent to the browser or stored in conversation history.

For the native Anthropic Messages API, set the provider kind and model/key-file pair instead:

```sh
JANUS_ARIA_PROVIDER_KIND=anthropic
JANUS_ARIA_ANTHROPIC_MODEL=your-model-id
JANUS_ARIA_ANTHROPIC_API_KEY_FILE=/absolute/path/to/anthropic-api-key
# Optional override; defaults to https://api.anthropic.com/v1/messages
JANUS_ARIA_ANTHROPIC_MESSAGES_URL=https://api.anthropic.com/v1/messages
```

The Anthropic key file must be a regular non-symlink file owned by the Janus runtime user, with owner-only permissions (0600) on Unix. The default provider kind is `openai-compatible`; partial provider configuration fails closed.

HTTPS is required for remote providers. Plain HTTP is accepted only for loopback providers such as a local Ollama-compatible service. Redirects are disabled, connection timeout is 10 seconds, request timeout is 60 seconds, and provider responses are capped at 2 MiB. No provider is selected or configured by this repository.

## Chat endpoint

- `POST /aria/chat` — authenticated, encrypted, provider-backed chat turn.

Request:

```json
{
  "conversation_id": "main",
  "message": "Explain the difference between a MOSFET and a BJT.",
  "request_id": "client-generated-unique-id",
  "include_private_context": false
}
```

The optional `request_id` makes retries idempotent: a completed request returns its saved assistant event instead of generating a second response. The response includes saved user/assistant event IDs, assistant content, provider label, and model ID.

The browser exposes an unchecked-by-default privacy toggle for private memory and knowledge retrieval. When `include_private_context` is true, retrieval is scoped to the authenticated owner and selected results are sent to the configured provider as untrusted reference data, capped at 12,000 characters. When false, no personal memory or knowledge records are retrieved for the provider. The current conversation messages are sent to the provider in either case. The UI clearly labels the legacy WebSocket path as scripted demo mode and does not silently fall back to it after sign-in.

The runtime saves the user message before provider dispatch and saves the assistant response before returning success. If the provider fails, the user message remains saved and the endpoint returns HTTP 502; retry using the same request ID. Only one chat turn runs at a time per runtime to prevent concurrent turns from interleaving.

## Authentication and ownership

Every endpoint below requires the existing Janus bearer session:

```http
Authorization: Bearer <session-token>
```

The runtime derives the owner ID from the authenticated account. No endpoint accepts an owner ID from the request body or query string.

## Endpoints

- `GET /aria/status` — report provider, encrypted-history, and private-memory readiness without exposing credentials.\n- `GET /aria/conversations?limit=20` — list the authenticated user's conversations (maximum 100).
- `GET /aria/conversations/{conversation_id}/events?limit=100` — read that owner's events (maximum 1,000).
- `POST /aria/conversations/{conversation_id}/events` — append an owner-scoped user or assistant event. External clients cannot append privileged system/tool events. The conversation is created on first append.
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

`created_at` is optional and defaults to the runtime's current UTC time. The encrypted storage layer can represent `system`, `user`, `assistant`, or `tool` roles for runtime-owned chat turns, but the authenticated external append endpoint accepts only `user` and `assistant`. Event IDs are unique within an owner namespace, not globally.

## Remaining production gates

1. Add end-to-end API tests for missing/expired sessions, cross-owner access, unavailable key/provider configuration, idempotent retries, append/read/delete, and interrupted provider calls.
2. Add owner-facing retention, export, and deletion controls before production release.
3. Verify compatibility against the actual configured provider/model and perform a real end-to-end chat test.
4. Complete deployment hardening, monitoring, backup/restore drills, and user acceptance.
5. Tool execution, voice, and autonomous device actions are not implemented by this provider adapter and must receive separate permission/security designs.
