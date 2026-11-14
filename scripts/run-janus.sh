#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for command in cargo curl; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf 'Missing required command: %s\n' "$command" >&2
    exit 1
  fi
done

printf '[JANUS] Building runtime and web services...\n'
cargo build --bin janus-runtime --bin janus-web

RUNTIME_PID=""
WEB_PID=""

cleanup() {
  if [[ -n "$WEB_PID" ]] && kill -0 "$WEB_PID" 2>/dev/null; then
    kill "$WEB_PID" 2>/dev/null || true
    wait "$WEB_PID" 2>/dev/null || true
  fi
  if [[ -n "$RUNTIME_PID" ]] && kill -0 "$RUNTIME_PID" 2>/dev/null; then
    kill "$RUNTIME_PID" 2>/dev/null || true
    wait "$RUNTIME_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

printf '[JANUS] Starting protected runtime API...\n'
RUNTIME_ARGS=()
if [[ -n "${JANUS_SERIAL_PORT:-}" ]]; then
  RUNTIME_ARGS+=(--serial-port "$JANUS_SERIAL_PORT")
fi
case "${JANUS_HARDWARE_ENABLED:-true}" in
  0|false|FALSE|no|NO) RUNTIME_ARGS+=(--disable-hardware) ;;
esac
./target/debug/janus-runtime "${RUNTIME_ARGS[@]}" &
RUNTIME_PID=$!

ready=0
for attempt in $(seq 1 60); do
  if ! kill -0 "$RUNTIME_PID" 2>/dev/null; then
    printf '[JANUS] Runtime exited before becoming healthy.\n' >&2
    wait "$RUNTIME_PID" || true
    exit 1
  fi
  if curl --silent --show-error --fail http://127.0.0.1:8080/health >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done

if [[ "$ready" != "1" ]]; then
  printf '[JANUS] Runtime did not become healthy within 60 seconds.\n' >&2
  exit 1
fi

printf '[JANUS] Runtime healthy. Starting web dashboard on port 5000...\n'
./target/debug/janus-web &
WEB_PID=$!
wait "$WEB_PID"
