// Mob spawn points from the darkfallcompanion.se community API -> data/mobspawns.json
//   [{ id, name, difficulty, x, z, tile, mobs: [{ id, name, qty, difficulty }] }]   (game coords; main.js flips z for three.js)
// The original client never shipped monsterspawns.xml (server-side data), so positions come from the community site.
// Its map units are half-scale with y pointing the other way; the transform is fitted (least squares) against the NPC
// cities, which exist in both datasets:  x = ax*cx + bx,  z = az*cy + bz.
// Needs network access; run: node tools/build_mobspawns.js
const fs = require('fs'), path = require('path');
const BASE = 'https://darkfallcompanion.se/api';
const DIFFS = ['Very Easy', 'Easy', 'Medium', 'Hard', 'Very Hard', 'Boss', 'Champion'];
const get = async u => { const r = await fetch(BASE + u); if (!r.ok) throw new Error(u + ' -> ' + r.status); return r.json(); };

function fit(pairs) { // 1-D least squares: ours = a*theirs + b
  const n = pairs.length, sx = pairs.reduce((s, p) => s + p[0], 0), sy = pairs.reduce((s, p) => s + p[1], 0);
  const sxx = pairs.reduce((s, p) => s + p[0] * p[0], 0), sxy = pairs.reduce((s, p) => s + p[0] * p[1], 0);
  const a = (n * sxy - sx * sy) / (n * sxx - sx * sx), b = (sy - a * sx) / n;
  const rms = Math.sqrt(pairs.reduce((s, p) => s + (a * p[0] + b - p[1]) ** 2, 0) / n);
  return { a, b, rms };
}

(async () => {
  const world = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/world.json')));
  const xml = fs.readFileSync(path.join(__dirname, '../data/zones/dynamic_objects_npc_cities.xml'), 'latin1');
  const ours = {};
  for (const m of xml.matchAll(/<SFLevelObjectNpcCity>[\s\S]*?<\/SFLevelObjectNpcCity>/g)) {
    const [, x, , z] = m[0].match(/<SFVector3 X="([-\d.]+)" Y="([-\d.]+)" Z="([-\d.]+)"/).map(Number);
    ours[m[0].match(/<CityName>\s*<SFString Value="([^"]*)"/)[1].toLowerCase()] = { x, z };
  }
  const pois = (await get('/pois')).filter(p => p.type === 'NPC City' && ours[p.name.toLowerCase()]);
  const fx = fit(pois.map(p => [p.game_x, ours[p.name.toLowerCase()].x]));
  const fz = fit(pois.map(p => [p.game_y, ours[p.name.toLowerCase()].z]));
  console.log(`fit over ${pois.length} NPC cities: x = ${fx.a.toFixed(4)}*cx + ${fx.b.toFixed(1)} (rms ${fx.rms.toFixed(1)} m), z = ${fz.a.toFixed(4)}*cy + ${fz.b.toFixed(1)} (rms ${fz.rms.toFixed(1)} m)`);

  const spawns = await get('/spawns?' + DIFFS.map(d => 'difficulty=' + encodeURIComponent(d)).join('&'));
  const out = spawns.map(s => {
    const x = fx.a * s.game_x + fx.b, z = fz.a * s.game_y + fz.b;
    const tile = Object.keys(world).find(n => world[n].x0 <= x && x < world[n].x1 && world[n].z0 <= z && z < world[n].z1) || null;
    return {
      id: s.id, name: s.name, difficulty: s.difficulty, x: +x.toFixed(1), z: +z.toFixed(1), tile,
      mobs: s.mobs.map(m => ({ id: m.mob_id, name: m.mob_name, qty: m.quantity, difficulty: m.difficulty })),
    };
  }).sort((a, b) => a.id - b.id);
  console.log(`${out.length} spawns, ${out.filter(s => !s.tile).length} outside the mainland tiles`);
  fs.writeFileSync(path.join(__dirname, '../data/mobspawns.json'), JSON.stringify(out));
})().catch(e => { console.error(e); process.exit(1); });
