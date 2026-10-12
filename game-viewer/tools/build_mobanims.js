// Monster animation clips from the game's own keyframe files -> data/mobanims/<hash>.json + index.json { <model>: <hash> }  (the models named in
// data/mobmodels.json; models that share their animation sets share a file)
//   { dt, idle|walk: { n: <clip name>, nk, b: { <bone>: [tx, ty, tz, qx, qy, qz, qw] } } }
// Each of the 7 channels is a number (constant), an array of nk samples (dense) or { l: [lengths], c: [[coeffs]...] } (see evalChannel in js/mobanim.js).
//
// export/mesh/sfa/*.sfas ("SFAS") is a memory dump of C++ objects (pointer values are garbage), decoded here:
//   u32 version=2, u32 n_anims, then per animation
//     name (c-string) at +0, u32 n_bones at +0x80, the bone records start at +0x98
//   bone record (P = start):   c-string bone name at P,  f64 dt (1/30 s) at P+0x84 (+0x88 in a few files), u32 n_keys, u32 descriptor, then the channel data;
//   the next record starts right after the data (and so does the next animation: its name comes right after the last bone's data).
//   descriptor: one nibble per channel (tx ty tz qx qy qz qw, lowest nibble first):
//     0, 0xc   constant                                    one f32
//     1, 0xd   dense                                       n_keys f32
//     0xe, 0xa, 0x6   piecewise polynomial of degree 3, 2, 1:  u16 n_pieces, u8 length of the first piece (frames), then per piece (degree+1) f32 coefficients
//                  c0..c3 and, between pieces, a u8 length (frames) of the next piece.  value(frame) = sum c_k * (frame*dt)^k  -- GLOBAL time in seconds,
//                  not time since the piece started; piece i covers the frames up to the running sum of the lengths (sum = n_keys - 1).
//   Values are the bone's local transform in the game's left-handed space: the viewer negates z and flips the quaternion (-qx, -qy, qz, qw).
//   Quaternion components are fitted separately, so renormalise after evaluating.  The last frame equals the first (loops).
// Run: node tools/build_mobanims.js
const fs = require('fs'), path = require('path'), crypto = require('crypto');
const ROOT = path.join(__dirname, '../..'), OUT = path.join(__dirname, '../data/mobanims');
const DEG = { 0xe: 3, 0xa: 2, 0x6: 1 };

function cstr(d, o) {
  const e = d.indexOf(0, o); if (e < 0 || e - o > 64) throw new Error('no string at 0x' + o.toString(16));
  for (let i = o; i < e; i++) if (d[i] < 0x20 || d[i] > 0x7e) throw new Error('bad string at 0x' + o.toString(16));
  return d.toString('latin1', o, e);
}
function channels(d, o, nk, desc) {
  const ch = [], f = () => { const v = d.readFloatLE(o); o += 4; return Math.abs(v) < 1e-5 ? 0 : +v.toPrecision(6); };
  for (let c = 0; c < 7; c++) {
    const n = (desc >>> (4 * c)) & 15;
    if (n === 0 || n === 0xc) ch.push(f());
    else if (n === 1 || n === 0xd) ch.push(Array.from({ length: nk }, f));
    else if (DEG[n]) {
      const cnt = d.readUInt16LE(o), l = [d[o + 2]], coef = []; o += 3;
      for (let s = 0; s < cnt; s++) { coef.push(Array.from({ length: DEG[n] + 1 }, f)); if (s < cnt - 1) l.push(d[o++]); }
      ch.push({ l, c: coef });
    } else throw new Error('channel type ' + n.toString(16));
  }
  return [ch, o];
}
function parse(file) {
  const d = fs.readFileSync(file); if (d.toString('latin1', 0, 4) !== 'SFAS') throw new Error('not an SFAS file');
  const na = d.readUInt32LE(8), anims = {}; let pos = 12;
  for (let a = 0; a < na; a++) {
    const name = cstr(d, pos), nb = d.readUInt32LE(pos + 0x80); if (nb > 200) throw new Error('bone count');
    let P = pos + 0x98; const bones = {}; let nkAll = 0, dtAll = 0;
    for (let b = 0; b < nb; b++) {
      const bn = cstr(d, P); let so = 0, dt = 0;
      for (so of [0x84, 0x88, 0x80, 0x8c, 0x90]) { dt = d.readDoubleLE(P + so); if (dt > 0.005 && dt < 0.2) break; dt = 0; }
      if (!dt) throw new Error('no dt');
      const nk = d.readUInt32LE(P + so + 8), desc = d.readUInt32LE(P + so + 12); if (nk > 2000) throw new Error('n_keys');
      const [ch, end] = channels(d, P + so + 16, nk, desc);
      bones[bn] = ch; nkAll = nk; dtAll = dt; P = end;
    }
    anims[name] = { nk: nkAll, dt: dtAll, b: bones }; pos = P;
  }
  if (pos !== d.length) throw new Error('trailing data');
  return anims;
}

const PICK = {   // clip -> names to try, best first
  idle: ['normal_idle', 'neutral_idle', 'attackmode_idle', 'al_idle', 'idle_extra_01'],
  walk: ['neutral_walk_forward'],   // (neutral_run_forward would be the third; nothing here runs yet)
};
const cache = {};
const sfas = n => (cache[n] ??= (() => { try { return parse(path.join(ROOT, 'export/mesh/sfa', n)); } catch (e) { console.warn(n + ': ' + e.message); return null; } })());

fs.rmSync(OUT, { recursive: true, force: true }); fs.mkdirSync(OUT, { recursive: true });
const mm = JSON.parse(fs.readFileSync(path.join(__dirname, '../data/mobmodels.json')));
const models = [...new Set(Object.values(mm).flatMap(v => v.models))].sort();
let ok = 0, bytes = 0; const missing = [], index = {}, files = new Set();
for (const m of models) {
  const xf = path.join(ROOT, 'export/xml/animations', m + '.xml'); if (!fs.existsSync(xf)) { missing.push(m + ' (no xml)'); continue; }
  const x = fs.readFileSync(xf, 'latin1'), a = x.slice(x.indexOf('<Animations>'));
  const all = {};   // later sets override earlier ones
  for (const s of a.matchAll(/<Set\d+>\s*<SFString Value="([^"]+)"/g)) Object.assign(all, sfas(s[1]) || {});
  const out = {};
  for (const [k, names] of Object.entries(PICK)) {
    const n = names.find(n => all[n]); if (!n) continue;
    const { nk, dt, b } = all[n]; out.dt = dt; out[k] = { n, nk, b };
  }
  if (!out.idle && !out.walk) { missing.push(m + ' (no clips)'); continue; }
  const s = JSON.stringify(out), h = crypto.createHash('sha1').update(s).digest('hex').slice(0, 10); ok++;
  if (!files.has(h)) { files.add(h); bytes += s.length; fs.writeFileSync(path.join(OUT, h + '.json'), s); }
  index[m] = h;
}
fs.writeFileSync(path.join(OUT, 'index.json'), JSON.stringify(index));
console.log(`${ok}/${models.length} models, ${files.size} files, ${(bytes / 1e6).toFixed(1)} MB`); if (missing.length) console.log('missing:', missing.join(', '));
