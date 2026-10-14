#!/usr/bin/env node
// One-command import: builds game/data/ on this machine from the user's own, legally obtained copy of the game.
// Nothing under data/ is distributed with this project (it is git-ignored); this script is how it comes to exist.
//
//   node tools/import.js                  # everything except the (slow) navigation bake
//   node tools/import.js --nav            # ... plus server/data/nav (about 2 min, 280 MB)
//   node tools/import.js --offline        # skip the two steps that download community data (see below)
//   node tools/import.js --only world,items     # run just these steps (names are listed by --list)
//   node tools/import.js --list
//   (add --verbose to see everything the individual build scripts print)
//
// Inputs, both produced from your own game install by the unpacker/exporter (../darkfall-data-export, see its README):
//   ../export/      the unpacked game archive
//   ../converted/   models, terrain and maps converted to glTF
//
// Third-party data: mobspawns.json (monster spawn points) and pois.json (cities, villages, dungeons, ...) are NOT in the
// game's files. They are fetched from the community API of https://darkfallcompanion.se by tools/build_mobspawns.js and
// tools/build_pois.js, and remain that site's data; they are only downloaded to your machine, never redistributed here.
// Please respect the site's terms of use. --offline skips them (the server then runs without mobs / with default towns).
const fs = require('fs'), path = require('path'), { spawnSync } = require('child_process');

const ROOT = path.resolve(__dirname, '../..'), TOOLS = __dirname, DATA = path.resolve(__dirname, '../data');
const args = process.argv.slice(2), flag = f => args.includes(f);
const only = flag('--only') ? args[args.indexOf('--only') + 1].split(',') : null;

const script = (file, ...a) => () => {
  const r = spawnSync(process.execPath, [path.join(TOOLS, file), ...a], { stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 1 << 28, encoding: 'utf8' });
  const lines = (r.stdout || '').trim().split('\n');
  if (flag('--verbose')) console.log(lines.join('\n')); else if (lines[0]) console.log('   ' + lines[lines.length - 1].slice(0, 120));
  if (r.status !== 0) throw new Error(`${file} exited with ${r.status}`);
};

// The tiles that have a `<tile>_full.glb` (clan cities, walls, bind stones...): data/fulltiles.json, used by the viewer and build_nav.js
function fulltiles() {
  const world = JSON.parse(fs.readFileSync(path.join(DATA, 'world.json'), 'utf8'));
  const have = new Set(fs.readdirSync(path.join(ROOT, 'converted/maps')));
  const out = Object.keys(world).filter(n => have.has(n + '_full.glb')).sort();
  fs.writeFileSync(path.join(DATA, 'fulltiles.json'), JSON.stringify(out));
  console.log('fulltiles', out.length);
}

// name, needs network, what it does. Order matters: later steps read the files earlier ones wrote.
const STEPS = [
  ['keystreams', false, 'derive the unpacking keys from your game files (once, about 2 min)', script('make_keystreams.js')],
  ['unpack', false, 'unpack the game archive\'s XML and shader files into data/', script('unpack_all.js')],
  ['world', false, 'tile grid and terrain kinds (world.json)', script('build_world.js')],
  ['data', false, 'music, tiles, day/night environment, terrain textures', script('build_data.js')],
  ['fulltiles', false, 'tiles with a _full model', fulltiles],
  ['spawns', false, 'capital city spawn points', script('build_spawns.js')],
  ['paperdoll', false, 'character/armour composition', script('build_paperdoll.js')],
  ['props', false, 'light-emitting props', script('build_props.js')],
  ['veg', false, 'vegetation meshes', script('build_veg.js')],
  ['alphamode', false, 'material alpha modes', script('build_alphamode.js')],
  ['weapons', false, 'weapon table', script('build_weapons.js')],
  ['items', false, 'goods and weights', script('build_items.js')],
  ['vendors', false, 'shop placements', script('build_vendors.js')],
  ['combatfx', false, 'combat sounds and effects', script('build_combatfx.js')],
  ['playeranims', false, 'player animation clips', script('build_playeranims.js')],
  ['pois', true, 'places of interest   [darkfallcompanion.se]', script('build_pois.js')],
  ['mobspawns', true, 'monster spawn points   [darkfallcompanion.se]', script('build_mobspawns.js')],
  ['mobmodels', false, 'monster name -> model', script('build_mobmodels.js')],
  ['mobanims', false, 'monster animation clips', script('build_mobanims.js')],
  ['nav', false, 'navigation grids for the server (slow; needs --nav)', script('build_nav.js')],
];

if (flag('--list')) { for (const [n, net, d] of STEPS) console.log(n.padEnd(12), net ? '(network) ' : '          ', d); process.exit(0); }

const missing = ['export/xml-files', 'export/texture', 'converted/maps'].filter(p => !fs.existsSync(path.join(ROOT, p)));
if (missing.length) {
  console.error(`Cannot find ${missing.map(p => '../' + p).join(', ')} (looked in ${ROOT}).\n` +
    `Unpack and convert your own copy of the game first. The easy way is  ./play.sh /path/to/game/data  in the repository root, or by hand with the exporter (https://github.com/ddarbs/darkfall-data-export):\n` +
    `  python3 dfextract_3.py "<game>/data" export && python3 sfexport.py all export converted`);
  process.exit(1);
}

let failed = 0;
for (const [name, net, what, run] of STEPS) {
  if (only ? !only.includes(name) : (name === 'nav' && !flag('--nav')) || (net && flag('--offline'))) continue;
  console.log(`\n== ${name}: ${what}`);
  const t = Date.now();
  try { run(); console.log(`   ok (${((Date.now() - t) / 1000).toFixed(1)} s)`); }
  catch (e) {
    console.error(`   FAILED: ${e.message}`);
    if (net) console.error('   (this step needs network access; re-run it later with: node tools/import.js --only ' + name + ')');
    else { failed++; break; }   // the later steps depend on this one
  }
}
if (failed) process.exit(1);
console.log('\nImport finished. Start with: node serve.js   (and, in ../server, cargo run --release)');
