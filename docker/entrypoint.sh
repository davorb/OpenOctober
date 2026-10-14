#!/bin/bash
# Build the game-server, start it and the web server, and exit at RESTART_AT (06:00 local time) so Docker (restart: always)
# starts us again on whatever the host has pulled into /app by then.
set -u
cd /app/game-server
STATE=${STATE_DIR:-/state}; mkdir -p "$STATE"
RESTART_AT=${RESTART_AT:-06:00}

echo "[df] building game-server"
cargo build --release --bin game-server || { echo "[df] build failed"; exit 1; }

if [ -z "$(ls data/nav 2>/dev/null)" ] && [ -d ../converted ]; then
  echo "[df] baking nav data (once, ~2 min)"
  node ../game-viewer/tools/build_nav.js || echo "[df] nav bake failed, NPCs will run without it"
fi

NAV=data/nav; [ -z "$(ls data/nav 2>/dev/null)" ] && NAV=none

/cache/target/release/game-server --port 1338 --npcs "${NPCS:-500}" --nav "$NAV" \
  --db "$STATE/npcs.db" --accounts "$STATE/accounts.db" \
  --origins "${PUBLIC_ORIGIN:-https://matrix.davor.se}" &
GS=$!
BASE_PATH=${BASE_PATH:-/df} PORT=1337 GAME_SERVER=127.0.0.1:1338 node ../game-viewer/serve.js &
WEB=$!

# seconds until the next RESTART_AT
now=$(date +%s); target=$(date -d "$RESTART_AT" +%s); [ "$target" -le "$now" ] && target=$(date -d "tomorrow $RESTART_AT" +%s)
sleep $((target - now)) & TIMER=$!

stop() { kill -INT "$GS" 2>/dev/null; kill "$WEB" "$TIMER" 2>/dev/null; wait "$GS" 2>/dev/null; }  # SIGINT: the game-server saves the NPCs
trap 'stop; exit 0' TERM INT

wait -n "$GS" "$WEB" "$TIMER"
if kill -0 "$TIMER" 2>/dev/null; then echo "[df] a process died, exiting"; stop; exit 1; fi
echo "[df] $RESTART_AT: restarting"
stop; exit 0
