# Darkfall game-server

Multiplayer relay for the [game-viewer](../game-viewer): players pick a race/gender model, see each other move around the world and chat.
A technology demonstrator — no authentication, no anti-cheat, the server trusts whatever positions clients report.

```
cd game-server
cargo run --release                 # ws://0.0.0.0:1338
cargo run --release -- --port 1338 --radius 900 --max-players 256 --tick-hz 15
cargo run --release -- --spawns none     # no mobs (default: ../game-viewer/data/mobspawns.json)
cargo run --release --bin bots -- ws://127.0.0.1:1338 128      # load test: 128 wandering bots with chat (args: url count centre-x centre-z spread)
```
Then `node ../game-viewer/serve.js`, open http://localhost:1337/, pick a character and enter. The viewer connects to `ws://<page host>:1338`
(override with `?server=ws://host:port`); `?name=&race=&gender=` preselect the character. Without a server the viewer still works offline and keeps retrying.

## Design
* **One world task owns all state** (`src/world.rs`), so there are no locks. Connection tasks (`src/main.rs`) forward commands to it over a channel and
  each owns a bounded outbox the world writes into with `try_send`: a slow client just drops position snapshots, and is kicked if it can't keep up with events.
* **Relay, not simulation.** Clients are authoritative for their own position (x, y, z, yaw, speed, flags) and send it at 15 Hz. Every tick the server builds,
  per client, one binary snapshot of the players inside the **interest radius** (900 m by default; spatial hash with radius-sized cells, 3x3 lookup).
  Roster changes (join / leave, plus each player's name, race, gender and armour set) and chat are small JSON events sent to everybody, so avatars can be built before a player enters range.
* **Chat** is global (rate-limited, 200 chars, control characters stripped); the viewer also shows it as a bubble over the speaker's head when in view.
* **Wire format** is documented at the top of `src/proto.rs`. Positions are in the viewer's three.js space (game z negated); the server never interprets them beyond x/z distance.

## Mobs
`src/mobs.rs` loads `game-viewer/data/mobspawns.json` (built by `game-viewer/tools/build_mobspawns.js`) at start-up: 2,103 spawn points, ~13k mobs of 273 types.
The server owns them; they idle and wander within a 14 m leash of their spawn point (no combat, aggro or respawn yet). Clients get a one-off
`{"t":"mobtypes","types":[{id,name,difficulty}]}` after `welcome`, then `OP_MOBS` binary frames at tick-rate / 3 with the mobs inside the interest radius
(format in `src/proto.rs`; y is 0 with the snap flag, so the viewer drops them onto the terrain). A missing spawns file just logs a warning and runs without mobs.

## Living NPCs
`--npcs 500` adds server-run, labeled NPCs (`PlayerInfo::npc`, `FLAG_NPC`; ids from `0xC000`). They live in towns from `pois.json` / `spawns.json`, hop between neighbouring places, rest and wander (`src/npcs.rs`). Cost stays flat through simulation level of detail by distance to the nearest player: **active** (inside the interest radius: stepped every tick, sent in the snapshots), **near** (within 3 km: every 0.5 s) and **far** (every 5 s, travel is just progress along a leg). An NPC that gets close is caught up to "now" first, so it never jumps. NPC state (identity, level, home, needs, goal, position) is saved to SQLite (`--db npcs.db`, `--db none` to disable) every 30 s and on Ctrl-C, and restored at start.
With 500 NPCs and 128 clustered bots the tick average is unchanged (about 1.7 ms) - roughly 10 NPCs active, 30 near, 460 far. **Navigation.** `game-viewer/tools/build_nav.js` bakes one `data/nav/<tile>.nav` per land tile (2 m cells: too steep, water, walls of solid objects at body height; 4 m terrain heights; about 100 s and 280 MB for the whole world, generated and git-ignored). `--nav data/nav` (default) loads them lazily (`src/nav.rs`): active NPCs path-find with A* (250 m legs, string-pulled), stand at the real terrain height (no `FLAG_SNAP`) and never end a tick in a wall or lake; near and far NPCs still walk straight abstract lines, and one is only made visible from ground an avatar can stand on. **Skills, not levels.** NPCs have skills (0-100, named like the game's: Swords, Axes, Clubs and Maces, Greatswords, ..., Health) that are raised by use: every blow that lands trains the weapon skill and every blow taken trains Health, with gains that shrink once the skill is past what the mob is a fair fight for. Weapon skill sets their damage (0.6x to 1.4x), the quality of the weapon they carry and the toughest mob rank they dare hunt (`skill / 20`); Health sets their hit points. Skills are persisted (an older save with levels is dropped and regenerated).
**Hunting and combat.** An armed NPC with fatigue to spare sometimes picks a mob spawn near its town (`Goal::Hunt`), walks there and fights. Seen by a player it fights for real: weapon drawn (`equip`), `swing` clips, blows resolved by the same melee rules as players' (`melee_lands`), mobs aggro on it and hit back (`hit` with `v:"p"`), and a dead NPC leaves a corpse and gets up at home after 6 s. Out of sight a hunt is only time and training. Not yet: friendly and hostile reactions to players (players can't hurt NPCs and NPCs ignore players), chat, bows and spells for NPCs, tree trunks and in-tile lakes as obstacles.

## Measured (release build, Apple silicon, 128 bots all inside each other's radius)
~0.5 ms average / ~1.9 ms max per 66 ms tick, ~37 KB/s down per client (4.7 MB/s total out), no dropped snapshots. Cost grows with players x neighbours, so
the interest radius is what keeps it flat when players spread out.

## Not done (on purpose)
No player persistence (only NPCs are saved), no accounts, no server-side movement validation, no delta/quantised snapshots (20 bytes per entity is plenty at this scale), no TLS (put a proxy in front for `wss://`).
