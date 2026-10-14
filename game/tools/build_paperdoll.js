// Paperdoll (what a player model wears) -> data/paperdoll.json, from common/paperdoll/paperdollitems_<race> <gender>.xml and gamelogicinfo/attachpoints.xml
//   { chars: { hum_m: { body: [mesh item], attach: { "AttachPoint_Helmet": { frame, t:[x,y,z], r:[x,y,z] } }, sets: { "Chain": ["r_Armor Chain Arms", ...] },
//                       items: { "r_Armor Chain Arms": [mesh item] } } } }
//   mesh item: { f: mesh file ("NULL" = keep the body mesh, only the textures change), a: attach point (ATTACHED_MESHITEM) or null, s: subset ("Subset Torso"...),
//                m: [[materialId, texture name]] }
// The game composes a character from the "r_<Race> <Gender>" item (naked body + hair) and any worn items: a SUBSET item replaces the mesh/textures of that
// subset, an ATTACHED item hangs a static mesh on a skeleton frame. Textures are the .dds files in export/texture/dds (served at /dds/).
const fs = require('fs'), path = require('path');
const D = path.join(__dirname, '../data');
const RACES = { human: 'hum', alfar: 'alf', dwarf: 'dwf', ork: 'ork', mirdain: 'mir', mahirim: 'mhm' };
const norm = f => f.replace(/\\/g, '/');
const texName = t => norm(t).replace(/^.*\//, '').replace(/\.dds$/i, '');

function items(file) {
  const xml = fs.readFileSync(file, 'latin1'), out = {};
  for (const m of xml.matchAll(/<SFPaperdollItem ResourceName="([^"]+)">([\s\S]*?)<\/SFPaperdollItem>/g)) {
    const list = [];
    for (const mi of m[2].matchAll(/<SFMeshItem ([^>]*)>([\s\S]*?)<\/SFMeshItem>/g)) {
      const at = k => (mi[1].match(new RegExp(k + '="([^"]*)"')) || [])[1];
      const mats = [...mi[2].matchAll(/<SFMaterialShaderPair MaterialID="(\d+)"[^>]*>\s*<SFMaterialItem TextureName="([^"]+)"/g)].map(x => [+x[1], texName(x[2])]);
      list.push({ f: norm(at('FileName') || 'NULL'), a: at('AttachPointName') || null, s: at('SubsetName'), m: mats });
    }
    out[m[1]] = list;
  }
  return out;
}

// attach points: one set per animation file; the player skeleton frames (hum_f...) are shared, so any player set will do
const ap = fs.readFileSync(path.join(D, 'gamelogicinfo/attachpoints.xml'), 'latin1'), attach = {};
const sets = [...ap.matchAll(/<SFAttachPointSet>([\s\S]*?)<\/SFAttachPointSet>/g)].map(m => m[1]);
const set = sets.find(s => s.includes('animations/ork_m.xml')) || sets.find(s => s.includes('animations/alfar_f.xml'));
for (const m of set.matchAll(/<SFAttachPoint>([\s\S]*?)<\/SFAttachPoint>/g)) {
  const g = k => (m[1].match(new RegExp(`<${k}>\\s*<SFString Value="([^"]*)"`)) || [])[1];
  const v = k => { const x = m[1].match(new RegExp(`<${k}>\\s*<SFVector3 X="([-\\d.e]+)" Y="([-\\d.e]+)" Z="([-\\d.e]+)"`)); return x ? x.slice(1).map(Number) : [0, 0, 0]; };
  const name = g('SFAttachPointName'), frame = g('SFFrameName');
  if (name && frame && frame !== 'Undefined') attach[name] = { frame, t: v('SFTranslation'), r: v('SFRotation') };
}

const SLOTS = new Set(['Torso', 'Arms', 'Legs', 'Pants', 'Helmet', 'Shoulders', 'Elbows', 'Girdle', 'Boots', 'Footwear', 'Hands', 'Gauntlet', 'Gauntlets', 'Vambraces', 'Couters', 'Greaves', 'Bracers', 'Hood']);
const chars = {};
for (const [race, id] of Object.entries(RACES)) for (const [g, gi] of [['male', 'm'], ['female', 'f']]) {
  const all = items(path.join(D, `common/paperdoll/paperdollitems_${race} ${g}.xml`));
  const bodyName = Object.keys(all).find(n => n.toLowerCase() === `r_${race} ${g}`);
  const sets = {}, keep = {};
  for (const [n, list] of Object.entries(all)) {
    const w = n.split(' ');
    if (w[0] !== 'r_Armor' || !SLOTS.has(w[w.length - 1])) continue;
    keep[n] = list; (sets[w.slice(1, -1).join(' ')] ||= []).push(n);
  }
  chars[`${id}_${gi}`] = { body: all[bodyName], sets, items: keep };
}
const out = { attach, chars };
fs.writeFileSync(path.join(D, 'paperdoll.json'), JSON.stringify(out));
console.log('attach points', Object.keys(attach).length, '| characters', Object.keys(chars).length, '| armor sets (human male)', Object.keys(chars.hum_m.sets).length, '|', (JSON.stringify(out).length / 1024).toFixed(0), 'KB');
console.log(Object.keys(attach).join(', '));
