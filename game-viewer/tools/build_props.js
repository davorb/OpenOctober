// Per-tile placed light sources (torches, braziers, fireplaces, lamp poles, lanterns) and tree trunks (for collision),
// from converted/maps/<tile>.json.  -> data/props.json  { tile: { l: [x,y,z,kind,...], t: [x,z,...] } }   (three coords: z flipped)
//   kind 0 = open fire (firewood/fireplace/brazier/campfire/forge), 1 = torch, 2 = lamp / lantern / light pole
const fs = require('fs'), path = require('path');
const ROOT = path.resolve(__dirname, '../..');
const world = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/world.json')));
const FIRE = /fireplace|firewood|brazier|campfire|bonfire|forge|furnace|cauldron_fire/i, TORCH = /torch/i, LAMP = /lightpole|light_pole|lantern|_lamp|lamp_|lamppost|streetlight/i;
const out = {}; let nl = 0, nt = 0;
for (const [name, t] of Object.entries(world)) {
  if (!t.base) continue;
  let j; try { j = JSON.parse(fs.readFileSync(path.join(ROOT, 'converted/maps', name + '.json'))); } catch { continue; }
  const l = [], tr = [];
  for (const o of j.objects) {
    const n = o.xml.split('/').pop();
    const [x, y, z] = o.pos;
    if (o.type === 'r_Logging Spawn Tree' || o.type === 'r_Fruit Tree') { tr.push(+x.toFixed(1), +(-z).toFixed(1)); continue; }
    let k = -1, h = 0;
    if (TORCH.test(n)) { k = 1; h = 1.9; } else if (FIRE.test(n)) { k = 0; h = 1.0; } else if (LAMP.test(n)) { k = 2; h = 3.8; }
    if (k >= 0) l.push(+x.toFixed(1), +(y + h).toFixed(1), +(-z).toFixed(1), k);
  }
  if (l.length || tr.length) { out[name] = { l, t: tr }; nl += l.length / 4; nt += tr.length / 2; }
}
console.log('lights', nl, 'trees', nt);
fs.writeFileSync(path.join(__dirname, '../data/props.json'), JSON.stringify(out));
