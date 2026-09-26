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
resp=$( { printf '%s\n%s\n%s\n' "$init" "$note" "$list"; sleep 2; } | timeout 10 "$BIN" mcp || true)
echo "$resp" | grep -q send_request || { echo "mcp: tools/list has no send_request"; echo "$resp"; exit 1; }
echo "smoke test passed"
