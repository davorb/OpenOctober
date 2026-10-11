# Darkfall asset viewer

Browser viewer for the glTF/PNG output in `../converted`: maps, static models, characters (bind pose) and terrain tiles.

```
python3 serve.py          # http://localhost:8000/   (optional port argument)
python3 build_catalog.py  # only needed if ../converted changes; serve.py builds it on first run
```

`serve.py` serves this folder at `/` and `../converted` at `/data/`, so the GLBs' relative `../textures/x.png`
references resolve. three.js is loaded from the jsdelivr CDN (needs internet).

* **Maps**: variants Base / Full / Terrain only. Terrain tiles are rendered with a custom shader that blends the
  real `NN_tex_terrain_*` textures using the per-vertex splat weights stored in the tiles. "Tex size" sets the world units
  per texture repeat (the game's value isn't known, 16 is a guess). "Terrain tint" multiplies by the baked vertex colour.
* Info panel lists every texture the asset uses (click to enlarge) and warns about missing files.
* Links are shareable: `#maps/a05/full`, `#models/a05_stairs_01`.
