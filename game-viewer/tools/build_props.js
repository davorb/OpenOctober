// Per-tile placed light sources (torches, braziers, fireplaces, lamp poles, lanterns) and tree trunks (for collision),
// from converted/maps/<tile>.json.  -> data/props.json  { tile: { l: [x,y,z,kind,...], t: [x,z,...] } }   (three coords: z flipped)
//   kind 0 = open fire (firewood/fireplace/brazier/campfire/forge), 1 = torch, 2 = lamp / lantern / light pole
const fs = require('fs'), path = require('path');
const ROOT = path.resolve(__dirname, '../..');
const world = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/world.json')));
const FIRE = /fireplace|firewood|brazier|campfire|bonfire|forge|furnace|cauldron_fire/i, TORCH = /torch/i, LAMP = /lightpole|light_pole|lantern|_lamp|lamp_|lamppost|streetlight/i;
const out = {}; let nl = 0, nt = 0;
// real light height from the model's POSITION accessor bounds (glb): lamp poles carry the lantern at the top, hanging lanterns have their origin at the top
// and hang below it, torches/fires burn at/above the top of the model
const bbc = new Map();
function bbox(mesh) {
  const key = mesh.replace(/\.sfm$/i, '').toLowerCase();
  if (bbc.has(key)) return bbc.get(key);
  let r = null;
  try {
    const b = fs.readFileSync(path.join(ROOT, 'converted/models', mesh.replace(/\.sfm$/i, '.glb'))), j = JSON.parse(b.slice(20, 20 + b.readUInt32LE(12)));
    let lo = 1e9, hi = -1e9;
    for (const m of j.meshes || []) for (const p of m.primitives) { const a = j.accessors[p.attributes.POSITION]; if (a && a.min) { lo = Math.min(lo, a.min[1]); hi = Math.max(hi, a.max[1]); } }
    if (hi > lo) r = [lo, hi];
  } catch {}
  bbc.set(key, r); return r;
}
function lightY(n, k, mesh) {
  const bb = mesh && bbox(mesh); if (!bb) return k === 1 ? 1.9 : k === 0 ? 1.0 : 3.8;
  const [lo, hi] = bb, h = hi - lo;
  if (k === 2 && !/pole/i.test(n)) return (lo + hi) / 2;   // hanging lantern / lamp body
  return k === 0 ? hi + 0.1 * h : hi - (k === 2 ? 0.08 : 0.1) * h;
}
for (const [name, t] of Object.entries(world)) {
  if (!t.base) continue;
  let j; try { j = JSON.parse(fs.readFileSync(path.join(ROOT, 'converted/maps', name + '.json'))); } catch { continue; }
  const l = [], tr = [];
  for (const o of j.objects) {
    const n = o.xml.split('/').pop();
    const [x, y, z] = o.pos;
    if (o.type === 'r_Logging Spawn Tree' || o.type === 'r_Fruit Tree') { tr.push(+x.toFixed(1), +(-z).toFixed(1)); continue; }
    let k = -1;
    if (TORCH.test(n)) k = 1; else if (FIRE.test(n)) k = 0; else if (LAMP.test(n)) k = 2;
    if (k >= 0) l.push(+x.toFixed(1), +(y + lightY(n, k, o.mesh)).toFixed(1), +(-z).toFixed(1), k);
  }
  if (l.length || tr.length) { out[name] = { l, t: tr }; nl += l.length / 4; nt += tr.length / 2; }
}
console.log('lights', nl, 'trees', nt);
fs.writeFileSync(path.join(__dirname, '../data/props.json'), JSON.stringify(out));
