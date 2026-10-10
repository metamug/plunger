#!/usr/bin/env bash
# Drives the real Plunger window on a Mac with real keyboard and mouse events and records the screen.
# Needs: the release binary, python3, and Accessibility / screen-recording permission (a GitHub macOS
# runner has both). Writes OUT/clip.mov, OUT/markers.txt, OUT/geometry.txt and a few screenshots.
#
#   scripts/demo/record-macos.sh target/release/plunger out
set -euo pipefail

BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
OUT=${2:?output directory}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
mkdir -p "$OUT"
: > "$OUT/markers.txt"

now() { python3 -c 'import time; print(f"{time.time():.3f}")'; }
mark() { echo "$(now)|$1" >> "$OUT/markers.txt"; }
snap() { screencapture -x "$OUT/$1.png" || true; }
sys_events() { osascript -e 'tell application "System Events"' -e "$1" -e 'end tell'; }
key() { sys_events "keystroke \"$1\" using $2 down"; }
code() { sys_events "key code $1"; }
send_it() { sys_events "key code 36 using command down"; }
typing() {
  osascript -e 'tell application "System Events"' \
    -e "repeat with c in characters of \"$1\"" -e 'keystroke c' -e 'delay 0.045' -e 'end repeat' -e 'end tell'
}
# Coordinates are relative to the top-left of the Plunger window.
click() { sys_events "click at {$((WX + $1)), $((WY + $2))}"; }
move() { sys_events "set the position of the mouse to {$((WX + $1)), $((WY + $2))}" 2>/dev/null || true; }
pause() { sleep "$1"; }

# --- the world the demo happens in
python3 "$ROOT/scripts/shop-api.py" "$OUT/shop.log" >/dev/null 2>&1 &
SHOP=$!
export PLUNGER_DATA_DIR
PLUNGER_DATA_DIR=$(mktemp -d)
for _ in $(seq 1 50); do curl -sf http://127.0.0.1:18095/items >/dev/null && break; sleep 0.1; done

"$BIN" > "$OUT/app-stdout.txt" 2>&1 &
APP=$!
sleep 7
sys_events 'set frontmost of process "plunger" to true'
sleep 1

# where the window is, so clicks can be placed and the video cropped
GEOM=$(osascript -e 'tell application "System Events" to tell process "plunger" to get {position, size} of window 1' | tr -d ' ')
IFS=, read -r WX WY WW WH <<< "$GEOM"
SCREEN=$(osascript -e 'tell application "Finder" to get bounds of window of desktop' | tr -d ' ')
IFS=, read -r _ _ SW SH <<< "$SCREEN"
echo "$WX,$WY,$WW,$WH,$SW,$SH" > "$OUT/geometry.txt"
echo "window $WX,$WY ${WW}x${WH} on a ${SW}x${SH} screen"

# --- record
DURATION=52
START=$(now)
screencapture -v -k -V "$DURATION" "$OUT/clip.mov" 2>"$OUT/recorder-err.txt" &
REC=$!
sleep 1.5
if ! kill -0 "$REC" 2>/dev/null; then
  # an older screencapture without -k
  screencapture -v -V "$DURATION" "$OUT/clip.mov" 2>>"$OUT/recorder-err.txt" &
  REC=$!
  START=$(now)
  sleep 1.5
fi
echo "$START" > "$OUT/start.txt"

mark "open|Plunger: an API client for you and your AI agent"
snap s0-open
pause 1.5

# 1. a person sends a request
mark "person|You: send a request from the window"
click 560 134
pause 0.4
key a command
pause 0.2
typing "http://127.0.0.1:18095/items?per_page=3"
pause 0.6
send_it
pause 2.2
snap s1-person

# 2. an agent works over MCP: its calls appear in the window, live
mark "agent|Your AI agent calls the API over MCP. Every call shows up here, live"
python3 "$HERE/agent.py" "$BIN" 0.3 &
AGENT=$!
sleep 11
snap s2-agent
wait "$AGENT" 2>/dev/null || true
pause 0.6

# 3. what the agent saved: the token stays hidden
mark "variables|The token it saved stays hidden, from the agent and from the screen"
click 614 183
pause 2.4
snap s3-variables

# 4. open one of its calls: the placeholder stays, never the token
mark "headers|Requests keep {{placeholders}}, so a secret is never written down"
click 130 238
pause 0.8
click 450 183
pause 2.4
snap s4-headers

# 5. run it again, look at the response
mark "rerun|Re-run any of the agent's calls yourself with Cmd+Enter"
click 302 183
pause 0.4
send_it
pause 2.4
snap s5-rerun
mark "menus|Every action is in the menus, with its shortcut"
click 100 46
pause 1.4
snap s6-menu
code 53
pause 0.4
mark "end|"
pause 1.0

# --- finish
wait "$REC" 2>/dev/null || true
kill "$APP" "$SHOP" 2>/dev/null || true
ls -la "$OUT"
