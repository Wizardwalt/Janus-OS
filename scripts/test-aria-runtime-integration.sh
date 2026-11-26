#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RUNTIME_BIN="${JANUS_RUNTIME_BIN:-$ROOT/target/debug/janus-runtime}"
if [[ ! -x "$RUNTIME_BIN" ]]; then
  echo "Janus runtime binary not found: $RUNTIME_BIN" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
PORT=18080
BASE="http://127.0.0.1:$PORT"
DB_PATH="$TMP_DIR/janus.db"
KEY_FILE="$TMP_DIR/aria-data.key"
CONFIG="$TMP_DIR/janus-test-config.json"
RUNTIME_PID=""
SENTINEL="AUTOMATED_OMNIVAULT_TEST_SENTINEL_20261120"
ADMIN_EMAIL="aria-ci-owner@example.invalid"
ADMIN_PASSWORD="CI-only-aria-owner-password-2026"
SECOND_EMAIL="aria-ci-second-owner@example.invalid"
SECOND_PASSWORD="CI-only-second-owner-password-2026"
CONVERSATION_ID="omnivault_ci_snapshot_001"

mkdir -p "$TMP_DIR/results"
cleanup() {
  if [[ -n "$RUNTIME_PID" ]] && kill -0 "$RUNTIME_PID" 2>/dev/null; then
    kill "$RUNTIME_PID" 2>/dev/null || true
    wait "$RUNTIME_PID" 2>/dev/null || true
  fi
  # Preserve synthetic diagnostics even when an assertion fails mid-run.
  if [[ -f "$TMP_DIR/results/test-results.txt" ]]; then
    mkdir -p "$(dirname "${JANUS_TEST_RESULTS:-/tmp/janus-aria-runtime-test-results.txt}")"
    cp "$TMP_DIR/results/test-results.txt" "${JANUS_TEST_RESULTS:-/tmp/janus-aria-runtime-test-results.txt}" || true
  fi
  if [[ -f "$TMP_DIR/runtime.log" ]]; then
    cp "$TMP_DIR/runtime.log" "${JANUS_RUNTIME_LOG:-/tmp/janus-aria-runtime.log}" || true
  fi
  if [[ -f "$TMP_DIR/runtime-no-key.log" ]]; then
    cp "$TMP_DIR/runtime-no-key.log" "${JANUS_RUNTIME_NO_KEY_LOG:-/tmp/janus-aria-runtime-no-key.log}" || true
  fi
  if [[ "${KEEP_JANUS_TEST_DIR:-0}" != "1" ]]; then
    rm -rf "$TMP_DIR"
  else
    echo "Test data retained at $TMP_DIR"
  fi
}
trap cleanup EXIT

printf '%s' "$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')" > "$KEY_FILE"
chmod 600 "$KEY_FILE"

cat > "$CONFIG" <<JSON
{
  "system_mode": "normal",
  "db_path": "$DB_PATH",
  "plugin_dir": "$ROOT/plugins",
  "core_modules_dir": "$ROOT/core",
  "web_bind": "127.0.0.1",
  "web_port": $PORT,
  "audit_enabled": true,
  "audit_retention_days": 7,
  "hardware_enabled": false,
  "serial_port": null,
  "license_public_key": null,
  "allowed_origins": ["http://localhost:5000"]
}
JSON

start_runtime() {
  local key_mode="$1"
  if [[ "$key_mode" == "enabled" ]]; then
    JANUS_ARIA_DATA_KEY_FILE="$KEY_FILE" "$RUNTIME_BIN" --config "$CONFIG" --disable-hardware       > "$TMP_DIR/runtime.log" 2>&1 &
  else
    env -u JANUS_ARIA_DATA_KEY_FILE -u JANUS_ARIA_MEMORY_KEY_FILE       "$RUNTIME_BIN" --config "$CONFIG" --disable-hardware       > "$TMP_DIR/runtime-no-key.log" 2>&1 &
  fi
  RUNTIME_PID=$!

  for attempt in $(seq 1 90); do
    if curl --silent --fail "$BASE/health" > "$TMP_DIR/health.json"; then
      return 0
    fi
    if ! kill -0 "$RUNTIME_PID" 2>/dev/null; then
      echo "Janus runtime exited before becoming healthy." >&2
      cat "$TMP_DIR/runtime.log" "$TMP_DIR/runtime-no-key.log" 2>/dev/null || true
      return 1
    fi
    sleep 1
  done
  echo "Janus runtime did not become healthy within 90 seconds." >&2
  cat "$TMP_DIR/runtime.log" "$TMP_DIR/runtime-no-key.log" 2>/dev/null || true
  return 1
}

stop_runtime() {
  if [[ -n "$RUNTIME_PID" ]] && kill -0 "$RUNTIME_PID" 2>/dev/null; then
    kill "$RUNTIME_PID"
    wait "$RUNTIME_PID" 2>/dev/null || true
  fi
  RUNTIME_PID=""
}

http_code() {
  local method="$1" url="$2" token="$3" body_file="$4"
  shift 4
  if [[ -n "$token" ]]; then
    curl --silent --show-error --output "$body_file" --write-out '%{http_code}' \
      -X "$method" -H "Authorization: Bearer $token" -H 'Content-Type: application/json' "$@" "$url"
  else
    curl --silent --show-error --output "$body_file" --write-out '%{http_code}' \
      -X "$method" -H 'Content-Type: application/json' "$@" "$url"
  fi
}

assert_code() {
  local actual="$1" expected="$2" label="$3"
  if [[ "$actual" != "$expected" ]]; then
    echo "FAIL: $label (expected HTTP $expected, got $actual)" >&2
    cat "$TMP_DIR/response.json" 2>/dev/null || true
    exit 1
  fi
  echo "PASS: $label (HTTP $actual)" | tee -a "$TMP_DIR/results/test-results.txt"
}

start_runtime enabled

# Bootstrap a disposable CI-only organization and owner account.
code="$(http_code POST "$BASE/auth/bootstrap" "" "$TMP_DIR/response.json" \
  --data '{"organization_name":"Aria CI Test","email":"'"$ADMIN_EMAIL"'","password":"'"$ADMIN_PASSWORD"'"}')"
assert_code "$code" 201 "first-owner bootstrap"
jq -e '.status == "success" and .data.role == "organization_admin"' "$TMP_DIR/response.json" >/dev/null

# Authenticate and prove protected routes reject anonymous requests.
code="$(http_code GET "$BASE/aria/conversations/$CONVERSATION_ID/events" "" "$TMP_DIR/response.json")"
assert_code "$code" 401 "anonymous conversation read is rejected"

code="$(http_code POST "$BASE/auth/login" "" "$TMP_DIR/response.json" \
  --data '{"email":"'"$ADMIN_EMAIL"'","password":"'"$ADMIN_PASSWORD"'"}')"
assert_code "$code" 200 "owner login"
OWNER_TOKEN="$(jq -r '.data.session_token' "$TMP_DIR/response.json")"
OWNER_ID="$(jq -r '.data.user_id' "$TMP_DIR/response.json")"
test -n "$OWNER_TOKEN" && test "$OWNER_TOKEN" != "null"

code="$(http_code GET "$BASE/auth/me" "$OWNER_TOKEN" "$TMP_DIR/response.json")"
assert_code "$code" 200 "authenticated identity lookup"
jq -e --arg id "$OWNER_ID" '.data.user_id == $id' "$TMP_DIR/response.json" >/dev/null

# The payload shape matches the normalized events generated by the Android
# OmniVault -> Janus importer. Content is synthetic and must remain encrypted.

# External clients must not be able to append privileged system/tool events.
cat > "$TMP_DIR/privileged.json" <<JSON
{"event_id":"ov_ci_snapshot_privileged_0000","role":"system","content":"Untrusted system prompt fixture.","provider":"OmniVault / CI fixture","model":null,"created_at":"2026-11-20T09:59:00Z"}
JSON
code="$(http_code POST "$BASE/aria/conversations/$CONVERSATION_ID/events" "$OWNER_TOKEN" "$TMP_DIR/response.json" \
  --data-binary @"$TMP_DIR/privileged.json")"
assert_code "$code" 400 "external system-role append is rejected"
cat > "$TMP_DIR/metadata.json" <<JSON
{"event_id":"ov_ci_snapshot_0000_0000","role":"user","content":"Imported from OmniVault. Conversation title: CI synthetic transfer. Source provider: CI fixture.","provider":"OmniVault / CI fixture","model":null,"created_at":"2026-11-20T10:00:00Z"}
JSON
cat > "$TMP_DIR/message.json" <<JSON
{"event_id":"ov_ci_snapshot_0001_0000","role":"user","content":"$SENTINEL","provider":"OmniVault / CI fixture","model":null,"created_at":"2026-11-20T10:01:00Z"}
JSON

code="$(http_code POST "$BASE/aria/conversations/$CONVERSATION_ID/events" "$OWNER_TOKEN" "$TMP_DIR/response.json" \
  --data-binary @"$TMP_DIR/metadata.json")"
assert_code "$code" 201 "import conversation metadata event"

code="$(http_code POST "$BASE/aria/conversations/$CONVERSATION_ID/events" "$OWNER_TOKEN" "$TMP_DIR/response.json" \
  --data-binary @"$TMP_DIR/message.json")"
assert_code "$code" 201 "import synthetic conversation message"

# Deterministic event IDs make a retry safe: duplicate writes are rejected and
# the stored transcript remains exactly two events.
code="$(http_code POST "$BASE/aria/conversations/$CONVERSATION_ID/events" "$OWNER_TOKEN" "$TMP_DIR/response.json" \
  --data-binary @"$TMP_DIR/message.json")"
assert_code "$code" 400 "duplicate event retry is rejected"

code="$(http_code GET "$BASE/aria/conversations/$CONVERSATION_ID/events?limit=1000" "$OWNER_TOKEN" "$TMP_DIR/response.json")"
assert_code "$code" 200 "owner reads imported transcript"
jq -e --arg sentinel "$SENTINEL" '
  .status == "success" and (.data | length == 2) and
  .data[0].event_id == "ov_ci_snapshot_0000_0000" and
  .data[1].event_id == "ov_ci_snapshot_0001_0000" and
  .data[1].content == $sentinel and
  .data[1].created_at == "2026-11-20T10:01:00Z"
' "$TMP_DIR/response.json" >/dev/null

# Exercise cursor pagination used by the Android importer.
code="$(http_code GET "$BASE/aria/conversations/$CONVERSATION_ID/events?limit=1" "$OWNER_TOKEN" "$TMP_DIR/response.json")"
assert_code "$code" 200 "first event page"
CURSOR="$(jq -r '.data[0].event_id' "$TMP_DIR/response.json")"
code="$(http_code GET "$BASE/aria/conversations/$CONVERSATION_ID/events?limit=1&after_event_id=$CURSOR" "$OWNER_TOKEN" "$TMP_DIR/response.json")"
assert_code "$code" 200 "cursor continuation page"
jq -e --arg sentinel "$SENTINEL" '.data | length == 1 and .[0].content == $sentinel' "$TMP_DIR/response.json" >/dev/null

# Create a second authenticated account in the same organization. It must not
# see the first owner's conversation, even when it knows the conversation ID.
code="$(http_code POST "$BASE/auth/users" "$OWNER_TOKEN" "$TMP_DIR/response.json" \
  --data '{"email":"'"$SECOND_EMAIL"'","password":"'"$SECOND_PASSWORD"'","role":"customer_read_only"}')"
assert_code "$code" 201 "second test account creation"
code="$(http_code POST "$BASE/auth/login" "" "$TMP_DIR/response.json" \
  --data '{"email":"'"$SECOND_EMAIL"'","password":"'"$SECOND_PASSWORD"'"}')"
assert_code "$code" 200 "second owner login"
SECOND_TOKEN="$(jq -r '.data.session_token' "$TMP_DIR/response.json")"
code="$(http_code GET "$BASE/aria/conversations/$CONVERSATION_ID/events?limit=1000" "$SECOND_TOKEN" "$TMP_DIR/response.json")"
assert_code "$code" 200 "second owner can query only its own namespace"
jq -e '.status == "success" and (.data | length == 0)' "$TMP_DIR/response.json" >/dev/null

# Inspect SQLite ciphertext, not application logs: the sentinel must not be
# present as plaintext in the encrypted conversation event database.
ARIA_DB="$TMP_DIR/janus.aria.sqlite"
python3 - "$ARIA_DB" "$SENTINEL" <<'PY'
import sqlite3, sys
path, sentinel = sys.argv[1], sys.argv[2].encode()
db = sqlite3.connect(path)
rows = db.execute("SELECT nonce, ciphertext FROM aria_conversation_events").fetchall()
assert len(rows) == 2, f"expected 2 encrypted events, found {len(rows)}"
assert all(sentinel not in bytes(nonce) + bytes(ciphertext) for nonce, ciphertext in rows), "plaintext sentinel found in ciphertext columns"
with open(path, "rb") as handle:
    contents = handle.read()
assert sentinel not in contents, "plaintext sentinel found in Aria SQLite database"
print("PASS: event contents are encrypted at rest (2 ciphertext records)")
PY
echo "PASS: encrypted-at-rest inspection" | tee -a "$TMP_DIR/results/test-results.txt"

# Restart with the same key and database; authenticated history must survive.
stop_runtime
start_runtime enabled
code="$(http_code POST "$BASE/auth/login" "" "$TMP_DIR/response.json" \
  --data '{"email":"'"$ADMIN_EMAIL"'","password":"'"$ADMIN_PASSWORD"'"}')"
assert_code "$code" 200 "owner can authenticate after runtime restart"
OWNER_TOKEN="$(jq -r '.data.session_token' "$TMP_DIR/response.json")"
code="$(http_code GET "$BASE/aria/conversations/$CONVERSATION_ID/events?limit=1000" "$OWNER_TOKEN" "$TMP_DIR/response.json")"
assert_code "$code" 200 "encrypted conversation survives runtime restart"
jq -e --arg sentinel "$SENTINEL" '.data | length == 2 and .[1].content == $sentinel' "$TMP_DIR/response.json" >/dev/null

# Restart without the data key. Conversation APIs must fail closed instead of
# silently creating plaintext storage or returning an empty successful result.
stop_runtime
start_runtime disabled
code="$(http_code POST "$BASE/auth/login" "" "$TMP_DIR/response.json" \
  --data '{"email":"'"$ADMIN_EMAIL"'","password":"'"$ADMIN_PASSWORD"'"}')"
assert_code "$code" 200 "owner login without Aria data key"
OWNER_TOKEN="$(jq -r '.data.session_token' "$TMP_DIR/response.json")"
code="$(http_code GET "$BASE/aria/conversations/$CONVERSATION_ID/events?limit=1000" "$OWNER_TOKEN" "$TMP_DIR/response.json")"
assert_code "$code" 503 "missing Aria data key fails closed"
jq -e '.status == "error" and (.error | contains("encrypted Aria conversation storage is unavailable"))' "$TMP_DIR/response.json" >/dev/null

echo "All Janus encrypted Aria runtime integration checks passed." | tee -a "$TMP_DIR/results/test-results.txt"
cp "$TMP_DIR/results/test-results.txt" "${JANUS_TEST_RESULTS:-/tmp/janus-aria-runtime-test-results.txt}"
cp "$TMP_DIR/runtime.log" "${JANUS_RUNTIME_LOG:-/tmp/janus-aria-runtime.log}" 2>/dev/null || true
