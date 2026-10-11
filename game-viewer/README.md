# Darkfall game-viewer

A fly-through of the whole Darkfall world in the browser: streamed terrain with the game's splat/triplanar material blending,
objects, water, day/night sky from the game's environment keyframes, and the music/ambience the game data assigns to each area.

```
node serve.js 8080        # http://localhost:8080/   (Chrome/Chromium; needs S3TC textures + Ogg)
```
Controls: **WASD** fly, **Shift** ×5, **Ctrl** ×0.2, **Space/E** up, **C/Q** down, **wheel** base speed, **T** pause time, **[ ]** ±30 min,
**V** atmosphere preset, **M** music, **N** noclip, **O** performance panel (presets + view/object distance + render scale), **H** help.
URL params: `?x=&z=&y=` (game coords), `?yaw=&pitch=`, `?t=` (minutes of day), `?view=`, `?obj=`, `?env=cc_ork`.

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
| start point | `worldconfig/worldconfig.xml` `SFDefaultPosition` | spawn |
| terrain shader | `shaders/roa_1d_terrain.teq` | `js/terrain.js`: triplanar 0.058 tiling, height-contest blend (sharp 3, bias .3), baked AO, shared detail map, world-continuous UVs |
| water shader | `shaders/roa_water.teq` | `js/water.js`: Gerstner constants, shallow/deep palette, sea level 215 |

Not done: the deep parts (>~40 KB) of the longest shader files decode poorly, so cascaded shadows, parallax and dynamic lights aren't ported;
grass/vegetation placement (`export/veg`) and the Java/native `classes` aren't decoded; music day-part boundaries follow the clear-sky keyframes.
