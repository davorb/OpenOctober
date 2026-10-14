#!/bin/bash
# Build the game-server, make sure the game data exists, start the game-server and the web server, and stop when either one stops.
set -u
cd /app/game-server
STATE=${STATE_DIR:-/state}; mkdir -p "$STATE"

if [ ! -f ../game-viewer/data/world.json ]; then
  if [ ! -d ../export ] || [ ! -d ../converted ]; then
    echo "[df] game-viewer/data is missing and so are export/ and converted/."
    echo "[df] Unpack and convert your own copy of the game first (run ./play.sh /path/to/game/data on the host once)."
    exit 1
  fi
  echo "[df] generating game-viewer/data from your game files"
  node ../game-viewer/tools/import.js || { echo "[df] import failed"; exit 1; }
fi

echo "[df] building game-server"
cargo build --release --bin game-server || { echo "[df] build failed"; exit 1; }

if [ -z "$(ls data/nav 2>/dev/null)" ] && [ -d ../converted ]; then
  echo "[df] baking nav data (once, a few minutes)"
  node ../game-viewer/tools/build_nav.js || echo "[df] nav bake failed, NPCs will run without it"
fi
NAV=data/nav; [ -z "$(ls data/nav 2>/dev/null)" ] && NAV=none

ORIGINS=(); [ -n "${PUBLIC_ORIGIN:-}" ] && ORIGINS=(--origins "$PUBLIC_ORIGIN")
/cache/target/release/game-server --port 1338 --npcs "${NPCS:-500}" --nav "$NAV" \
  --db "$STATE/npcs.db" --accounts "$STATE/accounts.db" "${ORIGINS[@]}" &
GS=$!
BASE_PATH=${BASE_PATH:-} PORT=1337 GAME_SERVER=127.0.0.1:1338 node ../game-viewer/serve.js &
WEB=$!

stop() { kill -INT "$GS" 2>/dev/null; kill "$WEB" 2>/dev/null; wait "$GS" 2>/dev/null; }  # SIGINT: the game-server saves the NPCs
trap 'stop; exit 0' TERM INT

wait -n "$GS" "$WEB"
echo "[df] a process exited, stopping"
stop; exit 1
