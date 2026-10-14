# Darkfall Web

> **This project does not include or distribute any copyrighted game assets. You must provide your own legally obtained data files.**

This repository contains only original source code: a browser client, a game server and a set of data-conversion tools.
It contains no game code, models, textures, audio, maps, text, or other content from any commercial game, and it never downloads such content for you.
Without your own lawfully obtained copy of the original game's data files, the software has nothing to display.

## Disclaimer

* **Not affiliated.** This is an independent, non-commercial fan and interoperability project. It is not affiliated with, authorised, sponsored or endorsed by the developers, publishers or rights holders of *Darkfall* or *Darkfall: Unholy Wars*. All trademarks and game names belong to their respective owners and are used here only to say which data format the software is compatible with.
* **Independent implementation.** The engine, client, server and network protocol are written from scratch. No original source code is used, and none may be contributed (see [CONTRIBUTING.md](CONTRIBUTING.md)).
* **Interoperability.** The tools exist so that people who already own the game can use their own data on modern systems with independently written software, as permitted for lawful users under Article 5(3) and Article 6 of the EU Software Directive (2009/24/EC; Sweden: upphovsrättslagen 26 h §). They are not a substitute for the original product and not a way to get it.
* **No piracy.** Please do not open issues or pull requests that ask for, link to, or share game files, download sites, torrents or cracked copies. They will be closed and removed.
* **No assets in the repo, ever.** Do not commit game files, converted files, screenshots of game content or anything under `export/`, `converted/` or `game-viewer/data/`. These folders are git-ignored on purpose.
* **Third-party data.** Two small data sets (monster spawn points and places of interest) are fetched at import time from the community API of [darkfallcompanion.se](https://darkfallcompanion.se). They are downloaded to your machine only, are not redistributed here and remain that site's data. Skip them with `--offline`.
* **Use at your own risk.** The software is provided as-is, without warranty. Make sure that your use of your own game copy is permitted by its licence and the laws where you live. This README is not legal advice.
* **Rights holders.** If you believe something in this repository infringes your rights, please contact the maintainers (open an issue or email the address on the maintainer's profile) before taking other steps, and we will look at it promptly.

## Quick start

```bash
./play.sh /path/to/your/game/data
```

`/path/to/your/game/data` is the folder of **your own copy** of the game that contains `dfdata.sfai` and the `dfdata_*.sfad` files. The script does everything else and opens <http://localhost:1337/> when the server is up:

1. fetches the separate data-exporter project into `darkfall-data-export/`
2. unpacks your archive into `export/` (about 14 GB) and converts it into `converted/`
3. derives the keys it needs to read your files (see "How your files are made readable" below) and builds the client's data in `game-viewer/data/`
4. compiles the game server and pre-computes NPC navigation grids
5. starts the game server and the web server

The first run takes a while (mostly steps 2 and 3, plus the Rust build). Later runs skip every finished step, remember the path in `.gamedata`, and just start the server: `./play.sh`. Stop it with Ctrl-C. `./play.sh --redo` makes everything again from scratch.

You need Python 3, Node.js 18+, Rust (stable), git, a Chromium-based browser, and about 20 GB free disk space. The steps below are the same thing done by hand, and there is a Docker option too.

## What is in the box

| folder | what |
| --- | --- |
| `darkfall-data-export/` | Not part of this repository: [a separate, public-domain tool](https://github.com/davorb/darkfall-data-export/tree/extract-more-files) that `play.sh` clones for you. It reads **your** game install, unpacks its archive and converts models, textures, terrain and maps to open formats (glTF/GLB, PNG, JSON). |
| [`game-viewer/`](game-viewer) | The browser client (three.js, plain JS) and the Node web server. Also the `tools/` that build `game-viewer/data/` from your converted files. |
| [`game-server/`](game-server) | Multiplayer server in Rust: accounts, chat, NPCs, mobs. See its [README](game-server/README.md). |
| [`docker/`](docker), [`docker-compose.yml`](docker-compose.yml) | One-command way to build and run everything. |
| [`web-viewer/`](web-viewer) | Small standalone model/asset browser for your converted files. |
| [`play.sh`](play.sh) | The one-command setup and launcher described above. |

## Requirements

* A **legally obtained copy of the game's data** that you have on disk (the folder containing its `dfdata.sfai` / `dfdata.sfad` archive). About 14 GB free disk space for the unpacked data, more for the converted files.
* Python 3 (standard library only) for the converter
* Node.js 18 or newer
* Rust (stable) for the server, **or** Docker instead of Node and Rust
* A Chromium-based browser (the client needs S3TC texture compression and Ogg playback)

## Doing it by hand

### 1. Make your game data readable

Everything in this step runs locally on your machine and reads only the files you point it at. Nothing is uploaded.

```bash
git clone -b extract-more-files https://github.com/davorb/darkfall-data-export.git
cd darkfall-data-export

# unpack the game archive into ../export  (~14 GB, takes a while)
python3 dfextract_3.py "/path/to/your/game/data" ../export

# convert models, textures, terrain, collision and maps  ->  ../converted
python3 sfexport.py all ../export ../converted
```

This produces the two folders the rest of the project expects next to the source code:

```
darkfall-web/
├── export/       unpacked archive (models, textures, sounds, XML, ...)
├── converted/    glTF/GLB models, PNG textures, terrain, collision, maps
├── game-viewer/
└── game-server/
```

Both folders are git-ignored. To convert a single piece instead of everything: `python3 sfexport.py maps ../export ../converted --name k18`.
Use `python3 -m http.server -d ../converted 8000` and `darkfall-data-export/viewer/` to inspect the converted files.

### 2. Build the client's data

The client does not read the raw game folders directly. One script turns `export/` and `converted/` into the compact JSON/binary files in `game-viewer/data/` (world grid, music, day/night keyframes, textures, spawn points, item tables, ...):

```bash
cd game-viewer
node tools/import.js              # about 15 s
```

Options:

| flag | effect |
| --- | --- |
| `--nav` | also bake the navigation grids the server's NPCs use (about 2 min, 280 MB) |
| `--offline` | do not download the community data from darkfallcompanion.se (the server then runs without mobs and with default towns) |
| `--only a,b` | run only the named steps |
| `--list` | list the steps |
| `--verbose` | show the output of each step |

`game-viewer/data/` is generated, git-ignored and must stay on your machine.

#### How your files are made readable

Part of the archive (the XML tables and shader sources) is stored obfuscated. The repository does not ship anything for that. `import.js` runs `tools/make_keystreams.js` first, which derives the needed byte streams statistically from the files in **your** `export/` folder (about 2 minutes, once) and keeps them in `game-viewer/.cache/keystreams/`, which is git-ignored. `unpack_all.js` then writes the readable files to `game-viewer/data/`. A tiny fraction of bytes (well below 0.1%) is recovered imperfectly, mostly in the first lines of some files and in the largest tables.

### 3. Run it

#### Option A: Docker

From the repository root, with `export/` and `converted/` in place:

```bash
docker compose up --build
```

The first start builds the server, generates `game-viewer/data/` if it is missing and bakes the navigation grids (a few minutes, once). Then open <http://localhost:1337/>. Accounts and NPC state are kept in `./state`.

Optional settings go in the environment or a `.env` file:

| variable | meaning |
| --- | --- |
| `DF_PORT` | port on the host (default 1337) |
| `DF_BIND` | address to listen on (default `127.0.0.1`; `0.0.0.0` to reach it from other machines) |
| `NPCS` | number of server-run NPCs (default 500) |
| `BASE_PATH` | serve under a path prefix behind a reverse proxy (the websocket on `<prefix>/ws` must pass through) |
| `PUBLIC_ORIGIN` | page origin(s) allowed to open the websocket, comma separated |

#### Option B: server and client separately

Start the game server (default `ws://0.0.0.0:1338`):

```bash
cd game-server
cargo run --release
# e.g. cargo run --release -- --npcs 500 --port 1338
```

Start the web server and client in a second terminal:

```bash
cd game-viewer
node serve.js
```

Open <http://localhost:1337/> in Chrome or Chromium, sign up (username, email, password) and create your character.
Without the game server the client still works as an offline explorer.

#### Administrator / free-fly explorer

```bash
cd game-server
cargo run --release -- --make-admin YOUR_USERNAME
```

Administrators can open `/admin.html`, a free-fly explorer with a spawn picker.

#### Controls (short)

| key | action |
| --- | --- |
| WASD / Space | move / jump (Shift sprint, Ctrl walk) |
| Enter | chat |
| Tab | online list |
| G | toggle fly and walk (explorer) |
| O | graphics and performance settings |
| H | full help |

See [`game-viewer/README.md`](game-viewer/README.md) for everything else.

## Troubleshooting

* **Black screen or missing terrain:** `game-viewer/data/` does not exist or is incomplete. Re-run `node tools/import.js` and check that `export/` and `converted/` are populated.
* **"export/ and converted/ missing" in Docker:** run `./play.sh` once (or step 1 by hand) on the host first; the container does not contain game files.
* **No sound or textures in Firefox/Safari:** use a Chromium-based browser.
* **No monsters or places of interest:** you used `--offline`, or darkfallcompanion.se was unreachable. Re-run `node tools/import.js --only pois,mobspawns`.

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) first. In short: write your own code, never copy or paste anything from original code or leaked material, and never add original assets to the repository.

## License

The source code of this project is licensed under the GNU General Public License v3.0 (see [`LICENSE`](LICENSE)). The license covers this project's code only. It grants no rights in any game, its data or its trademarks, which remain the property of their respective owners.
