#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

RUNTIME_BIN="$ROOT/target/release/janus-runtime"
if [[ ! -x "$RUNTIME_BIN" ]]; then
  echo "Janus runtime binary not found: $RUNTIME_BIN" >&2
  exit 1
fi

free_port() {
  python3 - <<'PY'
import socket
sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

PORT="$(free_port)"
PROVIDER_PORT="$(free_port)"
while [[ "$PROVIDER_PORT" == "$PORT" ]]; do
  PROVIDER_PORT="$(free_port)"
done
BASE="http://127.0.0.1:$PORT"
PROVIDER_BASE="http://127.0.0.1:$PROVIDER_PORT"
TMP_DIR="$(mktemp -d)"
DB_PATH="$TMP_DIR/janus.db"
KEY_FILE="$TMP_DIR/aria-data.key"
CONFIG="$TMP_DIR/janus-test-config.json"
RUNTIME_PID=""
PROVIDER_PID=""
EMAIL="aria-idempotency-ci@example.invalid"
PASSWORD="CI-only-aria-idempotency-password"

mkdir -p "$TMP_DIR/results"

cleanup() {
  if [[ -n "$RUNTIME_PID" ]] && kill -0 "$RUNTIME_PID" 2>/dev/null; then
    kill "$RUNTIME_PID" 2>/dev/null || true
    wait "$RUNTIME_PID" 2>/dev/null || true
  fi
  if [[ -n "$PROVIDER_PID" ]] && kill -0 "$PROVIDER_PID" 2>/dev/null; then
    kill "$PROVIDER_PID" 2>/dev/null || true
    wait "$PROVIDER_PID" 2>/dev/null || true
  fi
  mkdir -p test-results
  cp "$TMP_DIR/runtime.log" test-results/aria-idempotency-runtime.log 2>/dev/null || true
  cp "$TMP_DIR/provider.log" test-results/aria-idempotency-provider.log 2>/dev/null || true
  cp "$TMP_DIR/results/test-results.txt" test-results/aria-idempotency-results.txt 2>/dev/null || true
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

cat > "$TMP_DIR/mock_provider.py" <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import sys

class Handler(BaseHTTPRequestHandler):
    def respond(self, status, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path != "/count":
            self.respond(404, {"error": "not found"})
            return
        self.respond(200, {"count": self.server.request_count})

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.respond(404, {"error": "not found"})
            return
        length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(length)
        self.server.request_count += 1
        if self.server.request_count == 1:
            self.respond(503, {"error": "synthetic first-attempt outage"})
            return
        self.respond(200, {
            "model": "janus-ci-model",
            "choices": [{
                "message": {"content": "Aria idempotent retry succeeded."}
            }]
        })

    def log_message(self, _format, *_args):
        return

server = ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
server.request_count = 0
server.serve_forever()
PY

python3 "$TMP_DIR/mock_provider.py" "$PROVIDER_PORT" > "$TMP_DIR/provider.log" 2>&1 &
PROVIDER_PID=$!
for attempt in $(seq 1 30); do
  if curl --silent --fail "$PROVIDER_BASE/count" >/dev/null; then
    break
  fi
  sleep 1
done
curl --silent --fail "$PROVIDER_BASE/count" >/dev/null

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

JANUS_ARIA_DATA_KEY_FILE="$KEY_FILE" \
JANUS_ARIA_PROVIDER_CHAT_COMPLETIONS_URL="$PROVIDER_BASE/v1/chat/completions" \
JANUS_ARIA_PROVIDER_MODEL="janus-ci-model" \
JANUS_ARIA_PROVIDER_NAME="janus-ci-mock" \
  "$RUNTIME_BIN" --config "$CONFIG" --disable-hardware > "$TMP_DIR/runtime.log" 2>&1 &
RUNTIME_PID=$!

for attempt in $(seq 1 90); do
  if curl --silent --fail "$BASE/health" > "$TMP_DIR/health.json"; then
    break
  fi
  if ! kill -0 "$RUNTIME_PID" 2>/dev/null; then
    echo "Janus runtime exited before becoming healthy." >&2
    cat "$TMP_DIR/runtime.log" >&2
    exit 1
  fi
  sleep 1
done
curl --silent --fail "$BASE/health" >/dev/null

http_code() {
  local method="$1" url="$2" body_file="$3"
  shift 3
  curl --silent --show-error --output "$body_file" --write-out '%{http_code}' \
    -X "$method" "$@" "$url"
}

assert_code() {
  local actual="$1" expected="$2" label="$3"
  if [[ "$actual" != "$expected" ]]; then
    echo "FAIL: $label (expected HTTP $expected, got $actual)" >&2
    cat "$TMP_DIR/response.json" 2>/dev/null || true
    cat "$TMP_DIR/runtime.log" >&2 || true
    exit 1
  fi
  echo "PASS: $label (HTTP $actual)" | tee -a "$TMP_DIR/results/test-results.txt"
}

code="$(http_code POST "$BASE/auth/bootstrap" "$TMP_DIR/response.json" \
  -H 'Content-Type: application/json' \
  --data "{\"organization_name\":\"Aria Idempotency CI\",\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")"
assert_code "$code" 201 "bootstrap CI owner"

code="$(http_code POST "$BASE/auth/login" "$TMP_DIR/response.json" \
  -H 'Content-Type: application/json' \
  --data "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")"
assert_code "$code" 200 "authenticate CI owner"
OWNER_TOKEN="$(jq -r '.data.session_token' "$TMP_DIR/response.json")"
test -n "$OWNER_TOKEN" && test "$OWNER_TOKEN" != "null"

cat > "$TMP_DIR/chat.json" <<'JSON'
{"conversation_id":"aria_retry_ci_001","message":"Hello Aria","request_id":"retry-001"}
JSON

code="$(http_code POST "$BASE/aria/chat" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN" \
  -H 'Content-Type: application/json' \
  --data-binary @"$TMP_DIR/chat.json")"
assert_code "$code" 502 "first provider attempt fails but user message is persisted"
jq -e '.error | contains("retry with the same request_id")' "$TMP_DIR/response.json" >/dev/null

code="$(http_code GET "$BASE/aria/conversations/aria_retry_ci_001/events?limit=10" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN")"
assert_code "$code" 200 "read saved user message after provider failure"
jq -e '.data | length == 1 and .[0].role == "user" and .[0].content == "Hello Aria"' \
  "$TMP_DIR/response.json" >/dev/null

code="$(http_code POST "$BASE/aria/chat" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN" \
  -H 'Content-Type: application/json' \
  --data-binary @"$TMP_DIR/chat.json")"
assert_code "$code" 200 "retry same request ID after transient provider failure"
jq -e '.data.content == "Aria idempotent retry succeeded." and .data.provider == "janus-ci-mock"' \
  "$TMP_DIR/response.json" >/dev/null

code="$(http_code POST "$BASE/aria/chat" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN" \
  -H 'Content-Type: application/json' \
  --data-binary @"$TMP_DIR/chat.json")"
assert_code "$code" 200 "completed request ID returns saved assistant response"

jq -n --arg conversation "aria_retry_ci_001" --arg message "Different message" --arg request "retry-001" \
  '{conversation_id:$conversation,message:$message,request_id:$request}' > "$TMP_DIR/conflict.json"
code="$(http_code POST "$BASE/aria/chat" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN" \
  -H 'Content-Type: application/json' \
  --data-binary @"$TMP_DIR/conflict.json")"
assert_code "$code" 409 "completed request ID cannot be reused with different content"

provider_count="$(curl --silent --show-error "$PROVIDER_BASE/count" | jq -r '.count')"
test "$provider_count" = "2"
echo "PASS: first conversation called provider exactly twice (one failure, one successful retry)" | tee -a "$TMP_DIR/results/test-results.txt"

# Request IDs are scoped to a conversation. Reusing the same ID in a different
# conversation must create an independent turn rather than replaying old content.
cat > "$TMP_DIR/second-chat.json" <<'JSON'
{"conversation_id":"aria_retry_ci_002","message":"A separate conversation","request_id":"retry-001"}
JSON
code="$(http_code POST "$BASE/aria/chat" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN" \
  -H 'Content-Type: application/json' \
  --data-binary @"$TMP_DIR/second-chat.json")"
assert_code "$code" 200 "same request ID is independent in a different conversation"
jq -e '.data.conversation_id == "aria_retry_ci_002" and .data.content == "Aria idempotent retry succeeded."' \
  "$TMP_DIR/response.json" >/dev/null

code="$(http_code POST "$BASE/aria/chat" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN" \
  -H 'Content-Type: application/json' \
  --data-binary @"$TMP_DIR/second-chat.json")"
assert_code "$code" 200 "second conversation retry replays only its own saved response"

provider_count="$(curl --silent --show-error "$PROVIDER_BASE/count" | jq -r '.count')"
test "$provider_count" = "3"
echo "PASS: provider called exactly three times across two conversations; replay made no extra call" | tee -a "$TMP_DIR/results/test-results.txt"

code="$(http_code GET "$BASE/aria/conversations/aria_retry_ci_001/events?limit=10" "$TMP_DIR/response.json" \
  -H "Authorization: Bearer $OWNER_TOKEN")"
assert_code "$code" 200 "read completed idempotent conversation"
jq -e '
  .data | length == 2 and
  .[0].role == "user" and .[0].content == "Hello Aria" and
  .[1].role == "assistant" and .[1].content == "Aria idempotent retry succeeded."
' "$TMP_DIR/response.json" >/dev/null

echo "All Aria chat retry/idempotency integration checks passed." | tee -a "$TMP_DIR/results/test-results.txt"
