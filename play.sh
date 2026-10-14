#!/usr/bin/env bash
# One command from "I own the game" to "I can play it in my browser".
#
#   ./play.sh /path/to/your/game/data     first time: point it at the folder with YOUR copy's dfdata.sfai / dfdata_*.sfad
#   ./play.sh                             afterwards: remembers the path, skips every step that is already done
#
# Everything is built on this machine from your own files: the repository contains no game data (see README.md).
# Steps, each skipped when its result exists:
#   1. fetch the data exporter (a separate project) into ./darkfall-data-export
#   2. unpack your game archive              -> ./export       (about 14 GB)
#   3. convert models, textures, terrain     -> ./converted
#   4. derive the unpacking keys and build the client's data -> ./game/data
#   5. build the game server and bake NPC navigation grids
#   6. start the game server (:1338) and the web server (:1337), and open the browser
#
# Settings (environment): DF_PORT=1337  DF_BIND=127.0.0.1  NPCS=500  PUBLIC_ORIGIN=https://play.example.com  NO_OPEN=1
# Add --redo to throw away ./export ./converted ./game/data first and make them again.
#   ./play.sh --make-admin USERNAME           make an account an admin (in ./state/accounts.db), then exit
#   ./play.sh --set-password USERNAME:NEWPASS reset an account's password, then exit
set -euo pipefail
CALLER=$PWD
cd "$(dirname "$0")"
ROOT=$PWD
EXPORTER_URL=https://github.com/ddarbs/darkfall-data-export.git
EXPORTER_REF=main
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
die() { printf '\n\033[31m%s\033[0m\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not installed. $2"; }

REDO=0; GAME=${GAME_DATA:-}
ACCT=()
while [ $# -gt 0 ]; do a=$1; shift; case "$a" in
  --redo) REDO=1 ;;
  -h|--help) sed -n 2,19p "$0"; exit 0 ;;
  --make-admin|--set-password) [ $# -gt 0 ] || die "$a needs a value"; ACCT=("$a" "$1"); shift ;;
  /*) GAME=$a ;;
  *) GAME=$CALLER/$a ;;
esac; done
[ -z "$GAME" ] && [ -f .gamedata ] && GAME=$(cat .gamedata)

need python3 "Install Python 3."
need node "Install Node.js 18 or newer."
need cargo "Install Rust from https://rustup.rs (or use 'docker compose up --build' instead, see README.md)."
need git "Install git."
[ "$(node -p 'process.versions.node.split(".")[0]')" -ge 18 ] || die "Node.js 18 or newer is required."

if [ "$REDO" = 1 ]; then rm -rf export converted game/data game/.cache server/data/nav; fi

# 1 -------------------------------------------------------------------------------------------------------------------
if [ ! -f darkfall-data-export/sfexport.py ]; then
  say "fetching the data exporter"
  git clone --quiet -b "$EXPORTER_REF" "$EXPORTER_URL" darkfall-data-export
fi

# 2 -------------------------------------------------------------------------------------------------------------------
if [ ! -f export/.unpacked ]; then
  [ -n "$GAME" ] || die "Where is your copy of the game? Run:  ./play.sh /path/to/game/data
(the folder that contains dfdata.sfai and dfdata_0000.sfad ...). This project does not include any game files."
  [ -f "$GAME/dfdata.sfai" ] || die "$GAME/dfdata.sfai not found. Point play.sh at the data folder of your own legally obtained copy of the game."
  say "unpacking your game archive into ./export (this takes a while)"
  (cd darkfall-data-export && python3 dfextract_3.py "$GAME" "$ROOT/export")
  printf '%s\n' "$GAME" > .gamedata
  touch export/.unpacked
fi

# 3 -------------------------------------------------------------------------------------------------------------------
if [ ! -f converted/.converted ]; then
  say "converting models, textures, terrain and maps into ./converted (this takes a while)"
  (cd darkfall-data-export && python3 sfexport.py all "$ROOT/export" "$ROOT/converted")
  touch converted/.converted
fi

# 4 -------------------------------------------------------------------------------------------------------------------
if [ ! -f game/data/world.json ] || [ ! -f game/data/mobanims/index.json ]; then
  say "building the client's data (about 2 minutes; fetches community spawn/place data from darkfallcompanion.se)"
  node game/tools/import.js
fi

# 5 -------------------------------------------------------------------------------------------------------------------
say "building the game server"
(cd server && cargo build --release --bin server)
if [ -z "$(ls server/data/nav 2>/dev/null)" ]; then
  say "baking NPC navigation grids (once, a few minutes, ~280 MB)"
  node game/tools/build_nav.js || echo "nav bake failed, NPCs will run without it"
fi
NAV=data/nav; [ -z "$(ls server/data/nav 2>/dev/null)" ] && NAV=none

if [ ${#ACCT[@]} -gt 0 ]; then
  mkdir -p state
  (cd server && exec ./target/release/server --accounts "$ROOT/state/accounts.db" --capitals none "${ACCT[@]}")
  exit $?
fi

# 6 -------------------------------------------------------------------------------------------------------------------
PORT=${DF_PORT:-1337}; BIND=${DF_BIND:-127.0.0.1}
mkdir -p state
ORIGINS=(); [ -n "${PUBLIC_ORIGIN:-}" ] && ORIGINS=(--origins "$PUBLIC_ORIGIN")
say "starting: http://localhost:$PORT/   (Ctrl-C to stop)"
(cd server && exec ./target/release/server --port 1338 --npcs "${NPCS:-500}" --nav "$NAV" \
    --db "$ROOT/state/npcs.db" --accounts "$ROOT/state/accounts.db" ${ORIGINS[@]+"${ORIGINS[@]}"}) &
GS=$!
(cd game && exec env PORT="$PORT" BIND="$BIND" GAME_SERVER=127.0.0.1:1338 node serve.js) &
WEB=$!
stop() { kill -INT "$GS" 2>/dev/null || true; kill "$WEB" 2>/dev/null || true; wait "$GS" 2>/dev/null || true; }   # SIGINT: the server saves the NPCs
trap 'stop; exit 0' INT TERM
if [ -z "${NO_OPEN:-}" ]; then
  sleep 2
  if command -v open >/dev/null 2>&1; then open "http://localhost:$PORT/"; elif command -v xdg-open >/dev/null 2>&1; then xdg-open "http://localhost:$PORT/" >/dev/null 2>&1 || true; fi
fi
wait "$GS" "$WEB" || true
stop
