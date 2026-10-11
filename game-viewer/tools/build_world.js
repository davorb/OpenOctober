// Build data/world.json from zones.xml (tolerant of the ~0.02% corrupt bytes) + converted/maps
const fs = require('fs'), path = require('path');
const ROOT = path.resolve(__dirname, '../..');
const xml = fs.readFileSync(path.join(__dirname, '../data/zones/zones.xml'), 'latin1');
const maps = fs.readdirSync(path.join(ROOT, 'converted/maps'));
const size = n => { try { return fs.statSync(path.join(ROOT, 'converted/maps', n)).size; } catch { return 0; } };
const zones = xml.split('<Zone Name=').slice(1).map(z => {
  const name = (z.match(/^"([^"]*)"/) || [])[1];
  const levels = [...z.matchAll(/<Level Filename="([\w.]+)\.lvl" LevelID="(\d+)" MaxBBoxX="([-\d.]+)" MaxBBoxY="([-\d.]+)" MaxBBoxZ="([-\d.]+)" MinBBoxX="([-\d.]+)" MinBBoxY="([-\d.]+)" MinBBoxZ="([-\d.]+)"/g)]
    .map(m => ({ name: m[1], id: +m[2], max: [+m[3], +m[4], +m[5]], min: [+m[6], +m[7], +m[8]] }));
  return { name, levels };
});
const byZone = Object.fromEntries(zones.filter(z => z.levels.length > 20).map(z => [z.name || 'main', z.levels]));
console.log(Object.entries(byZone).map(([k, v]) => k + ':' + v.length));
const tiles = {};
for (const zn of ['main', 'border', 'full_agon']) for (const l of byZone[zn] || []) {
  if (tiles[l.name]) continue;
  if (!/^[a-z]{1,2}\d\d$|^end_of_world_tile/.test(l.name)) continue;   // dungeons / test levels share grid coordinates with real tiles
  const w = l.max[0] - l.min[0], d = l.max[2] - l.min[2];
  if (!(w > 100 && w < 1100 && d > 100 && d < 1100)) continue; // corrupt bbox
  tiles[l.name] = { x0: l.min[0], x1: l.max[0], z0: l.min[2], z1: l.max[2], zone: zn,
    base: size(l.name + '.glb'), empty: size(l.name + '_empty.glb'), full: size(l.name + '_full.glb') };
}
const LIQ = new Set(['r_Water', 'r_Lava', 'r_Swamp', 'r_Acid', 'r_Tar']);
for (const [n, t] of Object.entries(tiles)) {
  if (!t.base) continue;
  try { const j = JSON.parse(fs.readFileSync(path.join(ROOT, 'converted/maps', n + '.json')));
    const l = {}; let trees = 0;
    for (const o of j.objects) { if (LIQ.has(o.type)) l[o.xml.split('/').pop().replace(/\.xml$/, '')] = o.type; if (o.type === 'r_Logging Spawn Tree') trees++; }
    if (Object.keys(l).length) t.liquids = l; t.trees = trees; t.objects = j.objects.length; } catch {}
}
const have = Object.entries(tiles).filter(([, t]) => t.base || t.empty);
console.log('tiles', Object.keys(tiles).length, 'with converted geometry', have.length);
console.log(have.slice(0, 5));
fs.writeFileSync(path.join(__dirname, '../data/world.json'), JSON.stringify(Object.fromEntries(have)));

// ---- classify each tile's terrain: 'splat' (.ter with material weights) or 'mesh' (plain textured static mesh)
{
  const wj = path.join(__dirname, '../data/world.json');
  const w = JSON.parse(fs.readFileSync(wj));
  const kinds = {};
  for (const [n, t] of Object.entries(w)) {
    if (!t.empty) continue;
    const fd = fs.openSync(path.join(ROOT, 'converted/maps', n + '_empty.glb'), 'r');
    const head = Buffer.alloc(20); fs.readSync(fd, head, 0, 20, 0);
    const jb = Buffer.alloc(head.readUInt32LE(12)); fs.readSync(fd, jb, 0, jb.length, 20); fs.closeSync(fd);
    const j = JSON.parse(jb.toString());
    t.terrain = j.meshes[0]?.extras?.groups ? 'splat' : 'mesh';
    kinds[t.terrain] = (kinds[t.terrain] || 0) + 1;
  }
  console.log('terrain kinds', kinds, Object.entries(w).filter(([, t]) => t.terrain === 'mesh').map(([n, t]) => n + ':' + (t.empty / 1024 | 0) + 'k').join(' '));
  fs.writeFileSync(wj, JSON.stringify(w));
}
