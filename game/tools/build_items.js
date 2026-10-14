// Goods NPCs trade in, from gamelogicinfo/resourcetree.xml -> data/items.json   [{ id, cost, group, stack, weight }]
// `cost` is the game's SFBaseCostValue; `weight` is the "Weight" attribute of common/crafting/resourceattributeinfo.xml (absent when the game has none).
// Weapons that are in data/weapons.json but have no base cost are included with cost 0 (they can be carried and wielded, but not bought or sold). Groups: weapon (anything in data/weapons.json), armor, food, consumable, herb, material, ammo.
// The resource tree nests recipes, building pieces and clan items among the goods, so those are filtered out by category / name.
// The XML has a few corrupt bytes: read as latin1, unbalanced tags are skipped.
// run: node tools/build_items.js
const fs = require('fs'), path = require('path');
const xml = fs.readFileSync(path.join(__dirname, '../data/gamelogicinfo/resourcetree.xml'), 'latin1');
const weapons = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/weapons.json'), 'utf8')).weapons;
const BAD = /Buildables|Clan|recipe|Recipes|Figurine|Faction|Flags|Obelisk|House|Interior|Visual Features|Books|[\x00-\x08\x0b-\x1f]/;
const attrXml = fs.readFileSync(path.join(__dirname, '../data/common/crafting/resourceattributeinfo.xml'), 'latin1');
const weight = {};
for (const b of attrXml.matchAll(/<SFResourceAttributeInfo>[\s\S]*?<\/SFResourceAttributeInfo>/g)) {
  const name = b[0].match(/<SFResourceName>\s*<SFString Value="([^"]*)"/), w = b[0].match(/<SFAttributeName>\s*<SFString Value="Weight"\/>\s*<\/SFAttributeName>\s*<SFValue>\s*<SFFloat Value="([-\d.eE+]*)"/);
  if (name && w && isFinite(parseFloat(w[1]))) weight[name[1]] = parseFloat(w[1]);
}
const stack = [], items = [];
for (const m of xml.matchAll(/<SFListItem>|<\/SFListItem>|<SFName>\s*<SFString Value="([^"]*)"|<SFBaseCostValue>\s*<SFFloat Value="([-\d.eE+]*)"|<SFStackable>\s*<SFBool Value="(\w+)"/g)) {
  const t = m[0];
  if (t === '<SFListItem>') stack.push({ name: null, cost: null, stack: false });
  else if (t === '</SFListItem>') { const n = stack.pop(); if (n) items.push({ ...n, path: stack.map(s => s.name || '') }); }
  else if (m[1] !== undefined) { const top = stack[stack.length - 1]; if (top && top.name === null) top.name = m[1]; }
  else if (m[2] !== undefined) { const top = stack[stack.length - 1], v = parseFloat(m[2]); if (top && v > 0) top.cost = v; }
  else if (m[3] !== undefined) { const top = stack[stack.length - 1]; if (top) top.stack = m[3] === 'TRUE'; }
}
const group = i => {
  const near = i.path.slice(-3).join('/') + '/' + i.name, all = i.path.join('/');
  if (BAD.test(near)) return null;
  if (weapons[i.name]) return 'weapon';
  if (/r_Food/.test(all)) return 'food';
  if (/r_Consumables/.test(all)) return 'consumable';
  if (/r_Herbs/.test(all)) return 'herb';
  if (/r_Ammo/.test(all)) return 'ammo';
  if (/r_Materials/.test(all)) return 'material';
  if (/r_Armor/.test(all) && !/r_Armor Monsters/.test(all)) return 'armor';
  return null;
};
const out = [], seen = new Set();
for (const i of items) {
  if (!i.name || !i.name.startsWith('r_') || !i.cost || seen.has(i.name)) continue;
  const g = group(i);
  if (g) { seen.add(i.name); out.push({ id: i.name, cost: i.cost, group: g, stack: i.stack, ...(weight[i.name] !== undefined ? { weight: weight[i.name] } : {}) }); }
}
// every weapon of the game can be carried: those with no base cost are listed with cost 0
for (const id of Object.keys(weapons)) {
  if (!seen.has(id)) { seen.add(id); out.push({ id, cost: 0, group: 'weapon', stack: false, ...(weight[id] !== undefined ? { weight: weight[id] } : {}) }); }
}
out.sort((a, b) => a.id.localeCompare(b.id));
fs.writeFileSync(path.join(__dirname, '../data/items.json'), JSON.stringify(out));
const by = {}; for (const i of out) by[i.group] = (by[i.group] || 0) + 1;
console.log('with a weight:', out.filter(i => i.weight !== undefined).length);
console.log(out.length, 'items', by);
