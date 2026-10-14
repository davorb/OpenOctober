// Player animation clips from the game's own keyframe files -> data/playeranims/<hash>.json + index.json { "<race>_<m|f>": <hash> }
//   race: hum, alfar, dwarf, mahirim, mirdain, ork  (export/xml/animations/<race>_<m|f>.xml lists the .sfas files of each model: Set0..3 primary / mounts / secondary / emotes)
//   file: { dt, <clip>: { n, nk, b: { <bone>: [tx, ty, tz, qx, qy, qz, qw] } } }  -- same layout as data/mobanims (decoder: build_mobanims.js, evaluator: js/mobanim.js)
// *_primary.sfas clips drive the whole body (40 bones, locomotion per stance, jump, swim, dodge, death); *_secondary.sfas clips only the 31 upper-body bones
// (spine, arms, hands, neck, head: attacks, parries, stabs, bow/magic/throw, (un)sheathe) and are layered over the legs of the primary clip.  Clips that fail to
// decode (hum_m_secondary) are taken from the other gender of the same race: the clip names are identical.
// Run: node tools/build_playeranims.js
const fs = require('fs'), path = require('path'), crypto = require('crypto');
const { parse } = require('./build_mobanims.js');
const ROOT = path.join(__dirname, '../..'), OUT = path.join(__dirname, '../data/playeranims');
const PRIMARY = /^(normal|attackmode|attackmode_2handed|attackmode_halberd)_(idle|idle_01|walk_forward|walk_backward|run_forward|run_left|run_right|sprint_forward)$|^crouch_(idle|forward|backward|left|right|forward_left|forward_right|backward_left|backward_right)$|^(jump_start|jump_end|jump_01|falling|swim_idle|swim_forward|swim_sprint|gank_death|gank_idle_death|get_up|dodge_(singlehand|2handed|halberd)_(forward|backward|left|right))$/;
const SECONDARY = /^(singlehand|2handed|halberd|2daggers)_(attack_0[1-4]|vertical_attack_01|stab|wirlwind|parrying|knockback|seize)$|^combo_|^melee_disabling|^(unsheathe|sheathe)_|^(bow_|magic_|attack_throw)/;
const cache = {};
const sfas = n => (cache[n] ??= (() => { try { return parse(path.join(ROOT, 'export/mesh/sfa', n)); } catch (e) { console.warn(n + ': ' + e.message); return null; } })());
const files = m => [...fs.readFileSync(path.join(ROOT, 'export/xml/animations', m + '.xml'), 'latin1').matchAll(/Value="([a-z_0-9]+\.sfas)"/g)].map(x => x[1]);

fs.rmSync(OUT, { recursive: true, force: true }); fs.mkdirSync(OUT, { recursive: true });
const index = {}, written = new Set(); let bytes = 0;
for (const race of ['hum', 'alfar', 'dwarf', 'mahirim', 'mirdain', 'ork']) for (const g of ['m', 'f']) {
  const key = race + '_' + g, fl = files(key), out = {}; let dt = 0;
  const take = (file, re, tag) => { const a = file && sfas(file); if (!a) return false; for (const [n, c] of Object.entries(a)) if (re.test(n)) { out[n] = { n, nk: c.nk, b: c.b }; dt = c.dt; } return true; };
  take(fl.find(f => /_primary\.sfas$/.test(f)), PRIMARY);
  if (!take(fl.find(f => /_secondary\.sfas$/.test(f)), SECONDARY)) {
    const o = files(race + '_' + (g === 'm' ? 'f' : 'm')).find(f => /_secondary\.sfas$/.test(f)); console.log(key + ': secondary clips from', o); take(o, SECONDARY);
  }
  out.dt = dt;
  const s = JSON.stringify(out, (k, v) => typeof v === 'number' && !Number.isInteger(v) ? +v.toPrecision(5) : v), h = crypto.createHash('sha1').update(s).digest('hex').slice(0, 10);
  if (!written.has(h)) { written.add(h); bytes += s.length; fs.writeFileSync(path.join(OUT, h + '.json'), s); }
  index[key] = h; console.log(key, Object.keys(out).length - 1, 'clips', (s.length / 1e3).toFixed(0) + ' kB');
}
fs.writeFileSync(path.join(OUT, 'index.json'), JSON.stringify(index));
console.log(`${written.size} files, ${(bytes / 1e6).toFixed(1)} MB`);
