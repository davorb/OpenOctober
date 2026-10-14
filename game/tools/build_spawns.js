// Starting cities: the six racial capital bindstones in zones/dynamic_objects_bindstones.xml -> data/spawns.json
//   [{ name, race, x, y, z, tile }]   (game coords; main.js flips z for three.js)
// The Dawnshore tutorial bindstone is left out: its tile (ff32) is not part of the mainland world.
const fs = require('fs'), path = require('path');
const xml = fs.readFileSync(path.join(__dirname, '../data/zones/dynamic_objects_bindstones.xml'), 'latin1');
const world = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/world.json')));
const out = [];
for (const m of xml.matchAll(/<SFLevelObjectBindstone>[\s\S]*?<\/SFLevelObjectBindstone>/g)) {
  const b = m[0];
  const race = (b.match(/r_bindstone capital (\w+)/) || [])[1];
  if (!race) continue;
  const [, x, y, z] = b.match(/<Position>\s*<SFVector3 X="([-\d.]+)" Y="([-\d.]+)" Z="([-\d.]+)"/).map(Number);
  const name = b.match(/<SFName>\s*<SFString Value="([^"]*)"/)[1];
  const tile = Object.keys(world).find(n => world[n].x0 <= x && x < world[n].x1 && world[n].z0 <= z && z < world[n].z1) || null;
  out.push({ name, race: race[0].toUpperCase() + race.slice(1), x: +x.toFixed(1), y: +y.toFixed(1), z: +z.toFixed(1), tile });
}
out.sort((a, b) => a.race.localeCompare(b.race));
console.log(out);
fs.writeFileSync(path.join(__dirname, '../data/spawns.json'), JSON.stringify(out));
