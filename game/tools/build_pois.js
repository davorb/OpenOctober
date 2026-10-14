// Points of interest (cities, villages, hamlets, wilderness banks, fortresses, chaos cities, dungeons, ...) from the
// darkfallcompanion.se community API -> data/pois.json   [{ id, name, type, x, z, tile, notes }]  (game coords)
// Same fitted transform as tools/build_mobspawns.js (their map units are half-scale, y flipped; fitted against NPC cities).
// Needs network access; run: node tools/build_pois.js
const fs = require('fs'), path = require('path');
const BASE = 'https://darkfallcompanion.se/api';
const KEEP = ['NPC City', 'Player City', 'Player Hamlet', 'Village', 'Wilderness Bank', 'Chaos City', 'Fortress', 'Dungeon'];
const get = async u => { const r = await fetch(BASE + u); if (!r.ok) throw new Error(u + ' -> ' + r.status); return r.json(); };
function fit(pairs) {
  const n = pairs.length, sx = pairs.reduce((s, p) => s + p[0], 0), sy = pairs.reduce((s, p) => s + p[1], 0);
  const sxx = pairs.reduce((s, p) => s + p[0] * p[0], 0), sxy = pairs.reduce((s, p) => s + p[0] * p[1], 0);
  const a = (n * sxy - sx * sy) / (n * sxx - sx * sx), b = (sy - a * sx) / n;
  return { a, b, rms: Math.sqrt(pairs.reduce((s, p) => s + (a * p[0] + b - p[1]) ** 2, 0) / n) };
}
(async () => {
  const world = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/world.json')));
  const xml = fs.readFileSync(path.join(__dirname, '../data/zones/dynamic_objects_npc_cities.xml'), 'latin1');
  const ours = {};
  for (const m of xml.matchAll(/<SFLevelObjectNpcCity>[\s\S]*?<\/SFLevelObjectNpcCity>/g)) {
    const [, x, , z] = m[0].match(/<SFVector3 X="([-\d.]+)" Y="([-\d.]+)" Z="([-\d.]+)"/).map(Number);
    ours[m[0].match(/<CityName>\s*<SFString Value="([^"]*)"/)[1].toLowerCase()] = { x, z };
  }
  const all = await get('/pois');
  const ref = all.filter(p => p.type === 'NPC City' && ours[p.name.toLowerCase()]);
  const fx = fit(ref.map(p => [p.game_x, ours[p.name.toLowerCase()].x])), fz = fit(ref.map(p => [p.game_y, ours[p.name.toLowerCase()].z]));
  console.log(`fit over ${ref.length} NPC cities: rms ${fx.rms.toFixed(1)} / ${fz.rms.toFixed(1)} m`);
  const out = all.filter(p => KEEP.includes(p.type)).map(p => {
    const x = fx.a * p.game_x + fx.b, z = fz.a * p.game_y + fz.b;
    const tile = Object.keys(world).find(n => world[n].x0 <= x && x < world[n].x1 && world[n].z0 <= z && z < world[n].z1) || null;
    return { id: p.id, name: p.name, type: p.type, x: +x.toFixed(1), z: +z.toFixed(1), tile, notes: p.notes || undefined };
  }).sort((a, b) => a.id - b.id);
  const cnt = {}; out.forEach(p => cnt[p.type] = (cnt[p.type] || 0) + 1);
  console.log(`${out.length} pois`, cnt, `${out.filter(p => !p.tile).length} outside mainland tiles`);
  fs.writeFileSync(path.join(__dirname, '../data/pois.json'), JSON.stringify(out));
})().catch(e => { console.error(e); process.exit(1); });
