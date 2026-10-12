# Darkfall game-server

Multiplayer relay for the [game-viewer](../game-viewer): players pick a race/gender model, see each other move around the world and chat.
A technology demonstrator — no authentication, no anti-cheat, the server trusts whatever positions clients report.

```
cd game-server
cargo run --release                 # ws://0.0.0.0:9000
cargo run --release -- --port 9000 --radius 900 --max-players 256 --tick-hz 15
cargo run --release --bin bots -- ws://127.0.0.1:9000 128      # load test: 128 wandering bots with chat (args: url count centre-x centre-z spread)
```
Then `node ../game-viewer/serve.js 8080`, open http://localhost:8080/, pick a character and enter. The viewer connects to `ws://<page host>:9000`
(override with `?server=ws://host:port`); `?name=&race=&gender=` preselect the character. Without a server the viewer still works offline and keeps retrying.

## Design
* **One world task owns all state** (`src/world.rs`), so there are no locks. Connection tasks (`src/main.rs`) forward commands to it over a channel and
  each owns a bounded outbox the world writes into with `try_send`: a slow client just drops position snapshots, and is kicked if it can't keep up with events.
* **Relay, not simulation.** Clients are authoritative for their own position (x, y, z, yaw, speed, flags) and send it at 15 Hz. Every tick the server builds,
  per client, one binary snapshot of the players inside the **interest radius** (900 m by default; spatial hash with radius-sized cells, 3x3 lookup).
  Roster changes (join / leave / model / name) and chat are small JSON events sent to everybody, so avatars can be built before a player enters range.
* **Chat** is global (rate-limited, 200 chars, control characters stripped); the viewer also shows it as a bubble over the speaker's head when in view.
* **Wire format** is documented at the top of `src/proto.rs`. Positions are in the viewer's three.js space (game z negated); the server never interprets them beyond x/z distance.

## Measured (release build, Apple silicon, 128 bots all inside each other's radius)
~0.5 ms average / ~1.9 ms max per 66 ms tick, ~37 KB/s down per client (4.7 MB/s total out), no dropped snapshots. Cost grows with players x neighbours, so
the interest radius is what keeps it flat when players spread out.

## Not done (on purpose)
No persistence, no accounts, no server-side movement validation, no delta/quantised snapshots (20 bytes per entity is plenty at this scale), no TLS (put a proxy in front for `wss://`).
