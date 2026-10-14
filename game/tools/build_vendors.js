// Town vendors from zones/dynamic_objects_vendors.xml -> data/vendors.json   [{ name, kind, x, z }]  (game coords, like pois.json: wire z = -z)
// The file places one NPC per `<tile>_<name>_<kind>.xml` ("b22_serawen_merchant.xml"); only the kinds that trade goods are kept.
// The XML has a few corrupt bytes, so it is read as latin1 and entries are matched with regexes.
// run: node tools/build_vendors.js
const fs = require('fs'), path = require('path');
const KINDS = ['merchant', 'blacksmith', 'arcanist', 'market'];
const xml = fs.readFileSync(path.join(__dirname, '../data/zones/dynamic_objects_vendors.xml'), 'latin1');
const out = [];
for (const m of xml.matchAll(/<SFLevelObjectInteraction>[\s\S]*?<\/SFLevelObjectInteraction>/g)) {
  const p = m[0].match(/<SFVector3 X="([-\d.]+)" Y="([-\d.]+)" Z="([-\d.]+)"/), f = m[0].match(/<DescriptionFilename>\s*<SFString Value="([^"]+)"/);
  if (!p || !f) continue;
  const [, tile, rest] = f[1].replace(/\.xml$/, '').match(/^([a-z]+\d+)_(.+)$/) || [];
  if (!rest) continue;
  const parts = rest.split('_'), kind = parts[parts.length - 1];
  if (!KINDS.includes(kind)) continue;
  const [x, y, z] = [p[1], p[2], p[3]].map(Number);
  if (y < 150 || y > 500) continue; // dungeons and instances sit far below the surface
  const name = parts.slice(0, -1).join(' ').replace(/(^|[ -])([a-z])/g, (s, a, b) => a + b.toUpperCase()) || 'Vendor';
  out.push({ name, kind, x: +x.toFixed(1), z: +z.toFixed(1), tile });
}
out.sort((a, b) => a.tile.localeCompare(b.tile) || a.name.localeCompare(b.name));
fs.writeFileSync(path.join(__dirname, '../data/vendors.json'), JSON.stringify(out));
const by = {}; for (const v of out) by[v.kind] = (by[v.kind] || 0) + 1;
console.log(out.length, 'vendors', by);
