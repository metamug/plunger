#!/usr/bin/env bash
# Runs the built binary headlessly against a local server: CLI, refusal of an
# undefined variable, and an MCP handshake. Usage: scripts/smoke-cli.sh target/release/plunger
set -euo pipefail
BIN=$(realpath "${1:?path to the plunger binary}")
WORK=$(mktemp -d)
export PLUNGER_DATA_DIR="$WORK/data"
mkdir -p "$WORK/www"
echo '{"hello":"linux","n":7}' > "$WORK/www/ok.json"
python3 -m http.server 18080 --bind 127.0.0.1 --directory "$WORK/www" >/dev/null 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null || true; rm -rf "$WORK"' EXIT
for _ in $(seq 1 50); do curl -sf http://127.0.0.1:18080/ok.json >/dev/null && break; sleep 0.1; done

# Keep a deliberately loose startup budget: this is a regression tripwire for
# order-of-magnitude slowdowns, not a benchmark of shared CI runners.
# Milliseconds from python3: `date +%s%N` and `timeout` are GNU-only and missing on macOS.
now_ms() { python3 -c 'import time; print(int(time.time() * 1000))'; }
best_ms=
for _ in $(seq 1 3); do
  start_ms=$(now_ms)
  "$BIN" --version >/dev/null
  elapsed_ms=$(( $(now_ms) - start_ms ))
  if [ -z "$best_ms" ] || [ "$elapsed_ms" -lt "$best_ms" ]; then best_ms=$elapsed_ms; fi
done
echo "best startup: $best_ms ms"
if [ "$best_ms" -gt 2000 ]; then
  echo "warning: startup regression guard exceeded 2000 ms (best of 3: $best_ms ms)" >&2
fi

out=$("$BIN" send --url http://127.0.0.1:18080/ok.json --fail)
echo "$out"
echo "$out" | grep -q '"ok": *true'   || { echo "send: not ok"; exit 1; }
echo "$out" | grep -q '"hello": *"linux"' || { echo "send: body missing"; exit 1; }

set +e
"$BIN" send --url "http://127.0.0.1:18080/{{missing}}" >/dev/null
code=$?
set -e
[ "$code" = 2 ] || { echo "undefined variable: expected exit 2, got $code"; exit 1; }

"$BIN" history | grep -q ok.json || { echo "history did not record the request"; exit 1; }

init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
note='{"jsonrpc":"2.0","method":"notifications/initialized"}'
list='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
# The server stops when its input closes; a time limit is only a safety net where `timeout` exists
# (it does not on macOS).
limit=""
if command -v timeout >/dev/null 2>&1; then limit="timeout 10"; elif command -v gtimeout >/dev/null 2>&1; then limit="gtimeout 10"; fi
resp=$( { printf '%s\n%s\n%s\n' "$init" "$note" "$list"; sleep 2; } | $limit "$BIN" mcp || true)
echo "$resp" | grep -q send_request || { echo "mcp: tools/list has no send_request"; echo "$resp"; exit 1; }
echo "smoke test passed"
