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
**Hunting and combat.** An armed NPC with fatigue to spare sometimes picks a mob spawn near its town (`Goal::Hunt`), walks there and fights. Seen by a player it fights for real: weapon drawn (`equip`), `swing` clips, blows resolved by the same melee rules as players' (`melee_lands`), mobs aggro on it and hit back (`hit` with `v:"p"`), and a dead NPC leaves a corpse and gets up at home after 6 s. Out of sight a hunt is only time and training. **Where NPCs can be.** A visible NPC is only put down on open ground: a connected region of at least 1500 cells (`Nav::snap_open`), not inside a house, which is a sealed room in the baked walls. Destinations an NPC finds no route to (a basin ringed by steep ground and water) are remembered and not picked again for a while. `--npc-time-scale N` runs the time of NPCs nobody watches N times faster (visible ones always run in real time), to see NPC life without waiting.
**Players and NPCs.** With `--pvp on` (the default) a visible NPC that is armed treats players by side: the chaos races (Ork, Mirdain, Mahirim) attack players of the other three races within 28 m (an assumption, `npcs::chaos_race`: the game data only has numeric alignment), anybody fights back against a player who hurt it (a 90 s grudge), and friendly NPCs greet players of their own side from 14 m with a line of chat. Players hurt NPCs with melee and arrows like they hurt each other; a killed NPC leaves a corpse and gets up at home. With pvp off, NPCs and players ignore each other. An NPC in a fight with a player strikes at nobody else, and leaves the fight when the player is gone, down, 70 m away or the NPC is badly hurt.
**Exploring.** NPCs also go and look at things (`Goal::Explore`): the entrance of a dungeon within 3 km, or a spot of wilderness 400-1600 m from the place they are at, in an area (600 m square) they have not seen yet. They walk there at an easy pace, linger looking around for a while and remember the area (48 areas, not saved). A fixed per-NPC curiosity decides how often they do it, and an NPC that ended up far from its place (after exploring or hunting) usually walks back before choosing anything else. The stats line counts how many are exploring.
**Trade.** With `items.json` and `vendors.json` (built by `game-viewer/tools/build_items.js` / `build_vendors.js` from the game's resource tree and vendor placements; `--items none` turns it off) the towns have shops (`src/economy.rs`):
* **Vendors** are the ~200 merchants, blacksmiths, arcanists and market stalls the game places in towns (an NPC each, the first ids, gold name tag, `info.role == "vendor"`). They never leave their post and hold shelves of the goods their kind deals in (a blacksmith weapons and armour, an arcanist herbs and elixirs, a merchant food, ammo and materials) with a target stock and a purse. Prices are the game's base cost times a scarcity factor (an empty shelf asks up to 2.5x, a glut pays down to 0.5x), a vendor sells at 115% and buys at 55% of that, every unit is priced at the stock it finds, and shelves and purses drift back to their targets every two minutes.
* **Adventurers** keep what mobs drop (gold and a few goods by difficulty, invented: the game has no loot tables), sell it when they get back to town, buy meals (eaten when tired) and, from a blacksmith, the best weapon of their class their skill allows. They carry at most 40 goods.
* **Traders** (8% of the regular NPCs, `info.role == "trader"`, unarmed) buy where a good is cheap and carry it to a neighbouring town where the vendors would pay a fifth more than it cost, so prices even out over time.
* An NPC that is seen walks up to the vendor and stands at the counter a few seconds; far away the same trade is settled on arrival. Vendor shelves, purses and what NPCs carry are saved with the NPCs (a save is dropped if the number of vendors changed).
* **For the player's trading window:** `Npcs::vendors_near(x, z, r)` finds the counters in reach, `Npcs::economy()` holds the shelves, and `Vendor::{ask, bid, sell_to, buy_from}` do the trade against any `Stock` (gold plus goods), the same calls NPCs use. Players have no inventory or gold yet.
Not yet: chat replies, bows and spells for NPCs, NPCs fighting each other, tree trunks and in-tile lakes as obstacles.

## Measured (release build, Apple silicon, 128 bots all inside each other's radius)
~0.5 ms average / ~1.9 ms max per 66 ms tick, ~37 KB/s down per client (4.7 MB/s total out), no dropped snapshots. Cost grows with players x neighbours, so
the interest radius is what keeps it flat when players spread out.

## Not done (on purpose)
No player persistence (only NPCs are saved), no accounts, no server-side movement validation, no delta/quantised snapshots (20 bytes per entity is plenty at this scale), no TLS (put a proxy in front for `wss://`).
