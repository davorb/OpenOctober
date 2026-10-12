# Darkfall game-viewer

A fly-through of the whole Darkfall world in the browser: streamed terrain with the game's splat/triplanar material blending,
objects, water, day/night sky from the game's environment keyframes, and the music/ambience the game data assigns to each area.

```
node serve.js 8080        # http://localhost:8080/   (Chrome/Chromium; needs S3TC textures + Ogg)
```
Two modes, toggled with **G** (or the on-screen mode button): **Fly** (free camera) and **Walk** (gravity 9.81 m/s², collision with terrain, buildings, rocks and tree trunks, steps up ledges, swims; **Space** jumps; speeds from the game's `r_Player Races` entry (walk 2.5 / run 3.7 / sprint 5.5 m/s) × `PLAYER_SCALE` 1.5, because the architecture is ~1.5–2× human scale; W runs, **Shift** sprints, **Ctrl** walks. Change `PLAYER_SCALE` in `js/collision.js` to resize the player).
Controls: **WASD** fly, **Shift** ×5, **Ctrl** ×0.2, **Space/E** up, **C/Q** down, **wheel** base speed, **T** pause time, **[ ]** ±30 min,
**V** atmosphere preset, **M** music, **B** post-processing, **K** SSAO, **J** vegetation, **N** noclip, **O** performance panel (presets + view/object distance + render scale + shadows + dynamic lights; default Medium), **H** help, **P** parallax on/off.
URL params: `?x=&z=&y=` (game coords), `?yaw=&pitch=`, `?t=` (minutes of day), `?view=`, `?obj=`, `?env=cc_ork`.

## Multiplayer
Optional: with [`../game-server`](../game-server) running (`cargo run --release`, port 9000) the intro screen has a **character picker** (6 races x 2 bodies, live preview) and a name field; on entering, other players appear as avatars you can see walk around, with **name tags**, **chat bubbles**, a **radar** (top left) and an **online list** (**Tab**). **Enter** opens chat (Enter sends, Esc cancels). `?server=ws://host:9000` overrides the server, `?name=&race=hum|alf|dwf|ork|mir|mhm&gender=m|f` preselect the character. Handled by `js/multiplayer.js` (network, UI, interpolation) and `js/avatars.js`.
* Avatars are composed like the game's paperdoll (`tools/build_paperdoll.js` -> `data/paperdoll.json`, from `common/paperdoll/*.xml` + `attachpoints.xml`): the racial naked body (`converted/animated/<race>_<m|f>_<legs|torso|arms|head>.glb`, one shared skeleton) with the game's own skin textures (`export/texture/dds`, served at `/dds/`) and hair, optionally wearing one of the game's armour sets (Cloth ... Plate, Dragon, ...): armour swaps part meshes/textures and hangs helmets, shoulders, boots etc. on skeleton frames. A procedural walk cycle is driven by the other player's speed. Not ported: armour dye colour masks, the game's skin/metal shaders (plain PBR is used), character-creator face/hair variants (the default face and hair of each race are used). Each avatar costs roughly 10-25 draw calls depending on armour.
* **Mobs**: the server also streams its ~13k monsters (see [`../game-server`](../game-server)); `js/mobs.js` interpolates them, snaps them to the terrain and shows the game's own monster model (`converted/animated/<name>.glb`, bind pose with a small walking sway; no skeletal animation) with a name tag (coloured by difficulty) within 45 m and red radar dots. Model files within 300 m only. Spawn positions come from the community site darkfallcompanion.se (`tools/build_mobspawns.js` -> `data/mobspawns.json`; the original `monsterspawns.xml` was server-side and never shipped); mob name -> model comes from the game's own chain `resourcetree.xml` -> `export/xml/animations` -> `.sfam` (`tools/build_mobmodels.js` -> `data/mobmodels.json`, a few names without a resource are mapped by hand).
* Players further than the server's interest radius (900 m) aren't streamed; they stay in the online list as "out of range".

## Where the data comes from
Everything under `export/xml-files` (type 3 in the archive) and the shader sources are **encrypted with a per-class XOR keystream that
depends only on byte position**. `tools/solve_keystream.js` recovers it statistically (many-time-pad over ~2,900 files, byte-trigram model,
self-training) and `tools/decrypt_all.js` writes the plain files to `data/`. ~0.02% of bytes in the largest files are still wrong.

| need | game file | how it's used |
| --- | --- | --- |
| tile placement | `zones/zones.xml` (level bounding boxes, 1024 m grid, `a01`…`ff32`) | `tools/build_world.js` → `data/world.json` |
| place names | `gamelogicinfo/tileinfomap.xml` | HUD / banner |
| music | `zones/env_music_settings.xml` (area → tile list → Sunrise/Midday/Sunset/Midnight tracks) | `js/music.js` |
| ambience | `zones/env_audio_list.xml` + `audio/environmental` | forest/river/mountain wind loops, gusts, wolves |
| day cycle | `object/environment/*.xml` + `objectspace/environment/*.xml` | sun/moons, light, ambient, fog, clouds (`js/env.js`) |
| start point | `worldconfig/worldconfig.xml` `SFDefaultPosition` | default spawn |
| starting cities | `zones/dynamic_objects_bindstones.xml` (`r_bindstone capital <race>`) | intro-screen spawn picker (`tools/build_spawns.js` → `data/spawns.json`); `?spawn=Human` / `?spawn=Sanguine` preselects |
| terrain shader | `shaders/roa_1d_terrain.teq` | `js/terrain.js`: triplanar 0.058 tiling, height-contest blend (sharp 3, bias .3), baked AO, shared detail map, world-continuous UVs |
| terrain parallax | `roa_1d_terrain.teq` "PHASE 3a" | `js/terrain.js`: single-step, offset-limited (`toEye.xz/|toEye| * (h-0.5) * 0.020 * smoothstep(12, 3, dist)`), ground leg only, weights not re-derived. The game ships it **disabled** because the baked height payload (normal map `.r`) is a constant in the shipped files, so the height here is a proxy (albedo brightness). `?parallax=<strength>` (0 = off, default 1), **P** toggles |
| post-processing | `pp_brightpass`, `pp_gaussianh/v`, `pp_godray`, `pp_up4tonemapping` (CombineToneMap2), `pp_finalhdrl`, `pp_final_underwater` | `js/post.js`: scene → MSAA half-float target, bright-pass + 1/4-res 13-tap gaussian bloom, 32-tap god rays toward the sun (bright-pixel source, per-tap decay), extended-Reinhard tonemap (white cutoff 0.8) + vignette, underwater sine wobble/tint below sea level. **B** toggles, `?post=0` disables. SSAO (`roa_ssao.frag`'s 16-point sample sphere): half-res, depth rebuilt from the scene target's depth texture, normals from depth neighbours, radius 3 m, fades out 60–260 m, depth-aware blur; **K** toggles, `?ssao=0` disables (the game applies it to ambient light only; here it multiplies the whole lit frame). Not ported: `pp_grade` (its 64³ colour LUTs aren't in the extracted data), `pp_fxaa` (MSAA is used instead) |
| vegetation (approximate) | `export/veg/*.sfvm` tuft meshes, `terrain_<tile>.sfv` type tables, `shaders/vegetation.teq` | `tools/build_veg.js` → `data/veg.json`; `js/vegetation.js`: instanced tufts within 40 m of the camera (shrink-out 22–38 m), wind sway weighted by height, up-facing lighting from the vegetation shader. Placement is **not the game's**: the terrain worker derives per-class weights (green / dry / swamp / dead / flowers / forest floor) from the splat materials, and tufts are scattered deterministically per 8 m chunk where those are high, choosing only mesh types listed in that tile's `.sfv` table (weighted by its first float). **J** toggles, `?veg=0` disables |
| light coronas | `shaders/roa_lightglow.teq` | `js/lights.js`: additive round glow ((1-d)², warm rim → white core) on each active dynamic light, brighter at night |
| water shader | `shaders/roa_water.teq` | `js/water.js`: Gerstner constants, shallow/deep palette, sea level 215 |

Underground (Shoal's cavern): the terrain mesh has stacked surfaces (cave floor ≈153, ceiling ≈235, outer roof ≈364 over the city), so `terrain-worker.js` keeps every up-facing surface per grid point in `layers` for tiles that need it (`World.groundHeight(x, z, maxY)` picks the floor below you, `World.covered()` says whether terrain is overhead). Walk/fly clamping, swimming and the open-sea plane all use that, so you can enter the city and it isn't flooded by the ocean plane.

Not done: the deep parts (>~40 KB) of the longest shader files decode poorly, so the game's cascaded shadows aren't ported (shadows and dynamic lights are re-implemented, see below);
the real grass placement (the `(start,count)` run lists in `export/veg/terrain_*.sfv` — a 512-wide cell raster in at least `ff32`, not understood elsewhere — and the native scatter code, so `js/vegetation.js` approximates it) and the Java/native `classes` aren't decoded; music day-part boundaries follow the clear-sky keyframes.

## Shadows and lights
* One sun shadow map follows the camera (Low 1024 px / 120 m, Medium 2048 / 200 m, High 4096 / 300 m); terrain, objects and water-side props all cast and receive it. Off at night / low sun.
* Dynamic lights: the nearest N (4/8/16) of ~11k torches, braziers, fireplaces, lamp poles and lanterns (found by name in the level placements, `tools/build_props.js` -> `data/props.json`) light terrain and objects, flicker, and are strongest at night. The game's own `dynamic_objects_lights.bin` only covers instanced dungeons.
