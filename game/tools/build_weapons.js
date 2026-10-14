// Builds data/weapons.json: every wieldable item of the game's resource tree (weapons, shields, ammo) with its model, icon, stats and
// the animation / swing-sound families its skill group uses.
//   items   : gamelogicinfo/resourcetree.xml (needs keystream_tail.bin, see solve_tail.js) -> SFMeshFileName / SFIconFileName / SFLayerName,
//             tree position found from the pretty-printed indentation (a few closing tags are corrupt, so no XML parsing)
//   stats   : gamelogicinfo/weaponpropertylist.xml (reach m, angle deg, rank) - the only per-weapon numbers the client ships
//   names   : export/translations/en_gb.properties (display_name_<id>, desc_<id>)
//   groups  : client/skill/s_*.xml (Dynamic_Animation_*, se_weaponswing_*) + common/specialeffect/specialeffects.xml (se -> audio xml -> wav)
//   missiles: gamelogicinfo/missileinfolist.xml (arrows: velocity, range, effect)
// Output: { weapons: { "r_Talon Axe": {name, desc, kind, group, hands, reach, angle, rank, mesh, icon, src} }, groups: { greataxe: {skill, anim, swing, sounds[], hands} }, missiles: {...}, ammo: [...] }
// `src` says how the model was found: tree (node had it) | fuzzy (node name was corrupted, matched within 2 edits) | descriptor (waw ResourceType) | category (guessed from group).
const fs = require('fs'), path = require('path');
const ROOT = path.resolve(__dirname, '../../export'), D = path.resolve(__dirname, '../data'), CONV = path.resolve(__dirname, '../../converted/models');
const rd = p => fs.readFileSync(p).toString('latin1');
const clean = s => s == null ? null : (/[^\x20-\x7e]/.test(s) ? null : s);

// ---- resource tree (indentation based)
const lines = rd(path.join(D, 'gamelogicinfo/resourcetree.xml')).split('\n'), items = new Map(), stack = [];
for (let i = 0; i < lines.length; i++) {
  const m = lines[i].match(/^(\s*)<SFListItem>/); if (!m) continue;
  const ind = m[1].length; let j = i + 1, end = Math.min(lines.length, i + 80); const it = { ind, kv: {} };
  for (; j < end; j++) { if (/<SFListItem>/.test(lines[j]) && j > i) break; const k = lines[j].match(/^\s*<(SF\w+)>\s*$/); if (k) { const v = (lines[j + 1] || '').match(/Value="([^"]*)"/); if (v) it.kv[k[1]] = v[1]; } }
  const name = it.kv.SFName; if (!name) continue;
  while (stack.length && stack[stack.length - 1].ind >= ind) stack.pop();
  it.anc = stack.map(s => s.name); it.name = name; stack.push({ ind, name }); items.set(name, it);
}
console.log('tree items', items.size);

// ---- stats, names
const stats = {}; { const s = rd(path.join(D, 'gamelogicinfo/weaponpropertylist.xml')); const re = /<(SFMeleeWeapon|SFRangedWeapon|SFMageStaff)>([\s\S]*?)<\/\1>/g; let m;
  while ((m = re.exec(s))) { const g = t => { const x = m[2].match(new RegExp('<' + t + '>\\s*<SFFloat Value="([^"]*)"')); return x ? +x[1] : undefined; }, n = m[2].match(/Value="(r_[^"]*)"/); if (n) stats[n[1]] = { kind: { SFMeleeWeapon: 'melee', SFRangedWeapon: 'ranged', SFMageStaff: 'staff' }[m[1]], reach: g('SFWeaponReach'), angle: g('SFWeaponAngle'), rank: g('SFWeaponRank') }; } }
const tr = {}; for (const l of fs.readFileSync(path.join(ROOT, 'translations/en_gb.properties'), 'utf8').split('\n')) { const m = l.match(/^((?:display_name_|desc_)r_(?:\\.|[^=])*)=(.*)$/); if (m) tr[m[1].replace(/\\ /g, ' ')] = m[2].replace(/\\(.)/g, '$1').trim(); }

// ---- models
const wawDir = path.join(ROOT, 'xml/waw'), desc = {}; // basename -> {sfm, rtype}
(function w(d) { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); if (e.isDirectory()) { w(p); continue; } if (!e.name.endsWith('.xml')) continue; const s = rd(p); const sfm = (s.match(/<Normal>[\s\S]*?Value="([^"]*\.sfm)"/i) || [])[1], rt = (s.match(/<ResourceType Value="([^"]*)"/) || [])[1]; desc[e.name.slice(0, -4).toLowerCase()] = { sfm: sfm && sfm.slice(0, -4).toLowerCase(), rt }; } })(wawDir);
const glb = new Set(fs.readdirSync(CONV).filter(f => f.endsWith('.glb')).map(f => f.slice(0, -4)));
const byRT = {}; for (const [k, v] of Object.entries(desc)) if (v.rt && v.rt !== 'r_Generic Prop' && glb.has(v.sfm || k)) byRT[v.rt.toLowerCase()] = v.sfm || k;
const lev = (a, b) => { const d = Array.from({ length: a.length + 1 }, (_, i) => [i]); for (let j = 1; j <= b.length; j++) d[0][j] = j; for (let i = 1; i <= a.length; i++) for (let j = 1; j <= b.length; j++) d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1)); return d[a.length][b.length]; };
const glbNames = [...glb];
function modelOf(meshField) { // 'waw//racial_weapons//waw_sword_2handed_advanced_alfar_01.xml' -> glb basename (via descriptor .sfm), tolerating a corrupt char
  if (!meshField || meshField === 'NULL' || meshField === 'mesh_of_doom.xml') return null;
  const b = meshField.split('/').pop().replace(/\.xml$/i, '').toLowerCase(), d = desc[b], cand = (d && d.sfm) || b;
  if (glb.has(cand)) return cand; if (glb.has(b)) return b;
  let best = null, bd = 3; for (const g of glbNames) if (g.startsWith('waw_') && Math.abs(g.length - b.length) < 3) { const e = lev(g, b); if (e < bd) { bd = e; best = g; } } return best;
}
const GROUPS = [ // [group, regex on model basename]
  ['shield', /shield|buckler/], ['sithra', /sithra/], ['thrown', /grenade/], ['bow', /bow/], ['staff', /staff|wand|rod/], ['polearm', /halberd|polearm|spear|glaive|scythe/],
  ['knife', /dagger|knife|knives/], ['greatsword', /sword_2h|2handed.*sword|greatsword/], ['greataxe', /axe_2h|axe_2handed|greataxe/], ['greatclub', /warhammer_2h|club_2h|greatclub|maul/],
  ['sword', /sword/], ['axe', /axe/], ['club', /club|mace|hammer|flail|morning/]];
const TREE = { r_Axes: 'axe', r_Greataxes: 'greataxe', r_Swords: 'sword', r_Greatswords: 'greatsword', r_Clubs: 'club', r_Greatclubs: 'greatclub', r_Polearms: 'polearm', r_Daggers: 'knife', r_Knives: 'knife', 'r_Magic Staffs': 'staff', 'desc_r_Bows': 'bow', r_Bows: 'bow', r_Shields: 'shield' };
const groupOf = (model, it) => { if (model) for (const [g, re] of GROUPS) if (re.test(model)) return g; if (it) for (const a of [...it.anc].reverse()) if (TREE[a]) return TREE[a]; return null; };
const FALLBACK = { axe: 'waw_axe_1h_01', greataxe: 'waw_axe_2handed_001', sword: 'waw_sword001', greatsword: 'waw_sword_2handed_001', club: 'waw_club_1h_01', greatclub: 'waw_warhammer_2h_special_01', knife: 'waw_dagger_001', polearm: 'waw_halberd_special_01', bow: 'waw_bow_01', staff: 'waw_staff_special_01', shield: 'waw_shield001', sithra: 'waw_sithras_01' };
const HANDS = { axe: 1, sword: 1, club: 1, knife: 2, sithra: 1, shield: 1, thrown: 1, greatsword: 2, greataxe: 2, greatclub: 2, polearm: 2, bow: 2, staff: 2 };

// ---- weapons
const weapons = {}, names = Object.keys(stats), tn = [...items.keys()];
const fuzzy = n => { let best = null, bd = 2; for (const k of tn) if (Math.abs(k.length - n.length) < 3) { const e = lev(k, n); if (e < bd) { bd = e; best = k; } } return best; };
for (const id of names) {
  let it = items.get(id), src = 'tree'; if (!it || !clean(it.kv.SFMeshFileName)) { const f = fuzzy(id); if (f && items.get(f).kv.SFMeshFileName) { it = items.get(f); src = 'fuzzy'; } }
  let model = it && (modelOf(it.kv.SFMeshFileName) || modelOf(it.kv.SFIconFileName && it.kv.SFIconFileName.replace(/^.*resources\/+/, 'waw/').replace(/\.dds$/, '.xml'))); const rt = byRT[id.toLowerCase().replace(/^r_/, 'r_')];
  if (!model && rt) { model = rt; src = 'descriptor'; }
  if (!model && !/^r_Natural |^r_gm_/.test(id)) { const key = id.slice(2).toLowerCase().replace(/[^a-z]/g, ''), hit = glbNames.filter(g => g.startsWith('waw_') && g.replace(/[^a-z]/g, '').includes(key)); if (key.length > 4 && hit.length) { model = hit[0]; src = 'name'; } }
  const st = stats[id], group = groupOf(model, it) || (st.kind === 'ranged' ? 'bow' : st.kind === 'staff' ? 'staff' : null);
  const natural = /^r_Natural |^r_gm_/.test(id);
  // no model and no tree position: the weapon class is still readable from reach/angle (they are per class in the data) plus a hint from the name
  let g2 = group; if (!g2 && !natural) { const key = st.reach + '/' + st.angle, nm = id.toLowerCase(), hit = /hammer|club|smash|maul|masher|breaker/.test(nm);
    g2 = { '2.45/65': hit ? 'greatclub' : 'greataxe', '2.45/60': 'greatsword', '2.1/60': hit ? 'club' : /axe|pick/.test(nm) ? 'axe' : 'sword', '2.7/80': 'polearm', '1.6/70': 'knife', '2.2/60': 'sithra' }[key] || null; }
  if (!model && g2 && FALLBACK[g2] && glb.has(FALLBACK[g2])) { model = FALLBACK[g2]; src = 'category'; }
  const gfin = g2;
  weapons[id] = { name: tr['display_name_' + id] || id.slice(2), desc: tr['desc_' + id] || '', kind: st.kind, group: gfin, hands: HANDS[gfin] || 1, reach: st.reach, angle: st.angle, rank: st.rank, mesh: model, icon: it && clean(it.kv.SFIconFileName) ? it.kv.SFIconFileName.split('/').pop().replace(/\.dds$/, '') : null, src: model ? src : 'none', ...(natural ? { natural: 1 } : {}) };
}
// shields and ammo come from the tree layers
for (const it of items.values()) { const L = it.kv.SFLayerName; if (L !== 'LAYER_SHIELD' || weapons[it.name]) continue; const model = modelOf(it.kv.SFMeshFileName); if (!model) continue; weapons[it.name] = { name: tr['display_name_' + it.name] || it.name.slice(2), desc: tr['desc_' + it.name] || '', kind: 'shield', group: 'shield', hands: 1, mesh: model, icon: (it.kv.SFIconFileName || '').split('/').pop().replace(/\.dds$/, '') || null, src: 'tree' }; }
const ammo = [...items.values()].filter(i => i.kv.SFLayerName === 'LAYER_AMMUNITION').map(i => ({ id: i.name, name: tr['display_name_' + i.name] || i.name.slice(2), mesh: modelOf(i.kv.SFMeshFileName) })).filter(a => clean(a.id));

// ---- groups (skill -> animation, swing effect, sounds)
const SKILL = { axe: 's_axes', greataxe: 's_greataxes', sword: 's_swords', greatsword: 's_greatswords', club: 's_clubs and maces', greatclub: 's_greatclubs', knife: 's_knife mastery', polearm: 's_polearms', sithra: 's_sithras', bow: 's_archery', thrown: 's_throwing', shield: 's_parry' };
const FX = rd(path.join(D, 'common/specialeffect/specialeffects.xml'));
const audioIdx = new Map(); (function w(d, rel) { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const r = rel + '/' + e.name; if (e.isDirectory()) w(path.join(d, e.name), r); else audioIdx.set(r.toLowerCase(), r); } })(path.join(ROOT, 'audio'), '');   // real-case wav paths
const audioWav = x => { const p = path.join(ROOT, 'xml', x.replace(/\\/g, '/')); try { const w = (rd(p).match(/FileName="([^"]+\.wav)"/i) || [])[1]; return w && (audioIdx.get(('/' + w).replace(/^\/\//, '/').toLowerCase()) || w); } catch { return null; } };
const sounds = se => { const i = FX.indexOf('Value="' + se + '"'); if (i < 0) return []; const blk = FX.slice(i, FX.indexOf('</SFSpecialEffect>', i)), out = []; for (const m of blk.matchAll(/<SFXMLFileName\d*>\s*<SFString Value="([^"]+)"/g)) { const w = audioWav(m[1]); if (w) out.push(w); } return out; };
const groups = {};
for (const [g, sk] of Object.entries(SKILL)) { let s = ''; try { s = rd(path.join(D, 'client/skill', sk + '.xml')); } catch {} const anim = (s.match(/Value="(Dynamic_Animation_\w+)"/) || [])[1], swing = (s.match(/Value="(se_weaponswing_\w+)"/) || [])[1];
  groups[g] = { skill: sk, hands: HANDS[g], ...(anim ? { anim } : {}), ...(swing ? { swing, sounds: sounds(swing) } : {}) }; }
// skills whose xml has no (readable) Dynamic_Animation / swing effect: names from the skill tree / specialeffects
Object.assign(groups.knife, { anim: 'Dynamic_Animation_Knives', swing: 'se_weaponswing_knife', sounds: sounds('se_weaponswing_knife') });
groups.bow.anim = groups.bow.anim || 'bow'; groups.staff = { hands: 2, anim: 'magic' }; groups.thrown.anim = 'grenade';

// ---- missiles (arrows)
const missiles = {}; { const s = rd(path.join(D, 'gamelogicinfo/missileinfolist.xml')); for (const m of s.matchAll(/<SFMissileInfo>([\s\S]*?)<\/SFMissileInfo>/g)) { const v = t => (m[1].match(new RegExp('<' + t + '>\\s*<SF\\w+ Value="([^"]*)"')) || [])[1], n = v('SFName'); if (n && /^m_(Archery|Natural)/.test(n)) missiles[n] = { skill: v('SFSkillName'), res: v('SFResourceName'), v0: +v('SFStartVelocity') || undefined, vmax: +v('SFMaxVelocity') || undefined, range: +v('SFMaxDistanceTravel') || undefined, fx: (m[1].match(/Value="(se_[^"]+)"/) || [])[1] }; } }

// clip lengths (seconds) of the upper-body clips (hum_f_secondary.sfas; the other races' files have the same lengths): swing / draw durations for the client and the server
const clipDur = {}; { const a = require('./build_mobanims.js').parse(path.join(ROOT, 'mesh/sfa/hum_f_secondary.sfas')); for (const [n, c] of Object.entries(a)) clipDur[n] = +((c.nk - 1) * c.dt).toFixed(3); }
fs.writeFileSync(path.join(D, 'weapons.json'), JSON.stringify({ weapons, groups, missiles, ammo, clipDur }));
const c = {}; for (const w of Object.values(weapons)) c[w.src] = (c[w.src] || 0) + 1;
const gc = {}; for (const w of Object.values(weapons)) gc[w.group] = (gc[w.group] || 0) + 1;
console.log('weapons', Object.keys(weapons).length, 'by source', c, '\nby group', gc, '\nmissiles', Object.keys(missiles).length, 'ammo', ammo.length);
console.log('no model:', Object.entries(weapons).filter(([, w]) => !w.mesh).map(([k]) => k).join(', '));
console.log('groups', JSON.stringify(groups).slice(0, 1500));
