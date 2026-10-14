// The game's own combat effects (swing sounds, blood / impact splatters, arrow meshes and trails) -> data/combatfx.json { <se_name>: { cls, p: {SFField: value}, kids: [ ...same ] } }
// from common/specialeffect/specialeffects.xml (needs keystream_tail.bin: the effects live deep in the file).  Class names look like "EffectClass BloodSplatter";
// p holds every plain field (floats, ints, bools, strings, vectors as [x,y,z], colours as [r,g,b,a]); kids are the effects of its SFContainerManager (audio, lights, sub-particles).
// Audio effects list SFXMLFileName1..8 -> export/xml/audio/**.xml -> wav; those are resolved here to `wavs` (paths below export/audio, real case).
// Run: node tools/build_combatfx.js
const fs = require('fs'), path = require('path');
const ROOT = path.resolve(__dirname, '../../export'), D = path.resolve(__dirname, '../data');
const s = fs.readFileSync(path.join(D, 'common/specialeffect/specialeffects.xml')).toString('latin1');
const WANT = /^(se_impact_|se_bloodsplatter|se_blood_|se_arrow|se_general_arrow|se_weaponswing_|se_missile_fizzle|se_pe_hitsound|se_pe_hit_in_back|se_pe_arrow_load|se_blunt_weakness|se_deadeye_arrow|se_em_|se_fm_)/;

// audio xml -> wav, case-insensitive on both
const index = (dir) => { const m = new Map(); (function w(d, rel) { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const r = rel ? rel + '/' + e.name : e.name; if (e.isDirectory()) w(path.join(d, e.name), r); else m.set(r.toLowerCase(), r); } })(dir, ''); return m; };
const audioIdx = index(path.join(ROOT, 'audio')), xmlIdx = index(path.join(ROOT, 'xml/audio'));
const wavOf = x => {
  const k = x.replace(/\\/g, '/').replace(/^\/?audio\//i, '').toLowerCase(), f = xmlIdx.get(k); if (!f) return null;
  const m = fs.readFileSync(path.join(ROOT, 'xml/audio', f)).toString('latin1').match(/FileName="([^"]+\.wav)"/i); if (!m) return null;
  const a = audioIdx.get(m[1].replace(/\\/g, '/').replace(/^\//, '').toLowerCase()); return a ? '/' + a : null;
};

function props(b) {
  const p = {};
  for (const m of b.matchAll(/<(SF\w+)>\s*<SF(String|Float|Integer|Bool|Double|Long|Vector3|Color)\s+([^>]*?)\/?>/g)) {
    const [, k, t, a] = m; let v;
    if (t === 'Vector3') v = [...a.matchAll(/[XYZ]="([-\d.eE]+)"/g)].map(x => +x[1]);
    else if (t === 'Color') { const g = c => +(a.match(new RegExp(c + '="([-\\d.eE]+)"')) || [])[1]; v = [g('R'), g('G'), g('B'), g('A')]; }
    else { const x = (a.match(/Value="([^"]*)"/) || [])[1]; v = t === 'Bool' ? /true/i.test(x) : t === 'String' ? x : +x; }
    if (!(k in p)) p[k] = v;
  }
  return p;
}
const DROP = /^(SFClassName|SFMainEffect|SFRelationEffect|SFInheritDuration|SFOverWriteStartTimeWithCurrentTime|SFAffectCamera|SFMaxAffectCameraDistance|SFAffectCameraPower)$/;
// the file is pretty-printed: top-level effects start at 2 spaces of indentation, their container children deeper (a few closing tags are corrupt, so split by indentation)
function build(text) {
  const [head, ...rest] = text.split(/<SFContainerManager>/), p = props(head), e = { cls: String(p.SFClassName || '').replace(/^EffectClass /, ''), p: {} };
  for (const [k, v] of Object.entries(p)) if (!DROP.test(k)) e.p[k] = v;
  const wavs = []; for (let n = 1; n <= 8; n++) { const x = e.p['SFXMLFileName' + n]; if (x) { const w = wavOf(x); if (w) wavs.push(w); } delete e.p['SFXMLFileName' + n]; }
  if (wavs.length) e.wavs = wavs;
  const kids = rest.join('<SFContainerManager>').split(/<SFSpecialEffect>/).slice(1).map(build).filter(k => k.cls); if (kids.length) e.kids = kids; return e;
}
const out = {}; let n = 0;
for (const blk of s.split(/\n  <SFSpecialEffect>/).slice(1)) {
  n++; const name = (blk.match(/<SFName>\s*<SFString Value="([^"]*)"/) || [])[1];
  if (name && WANT.test(name)) { const e = build(blk); delete e.p.SFName; out[name] = e; }
}
fs.writeFileSync(path.join(D, 'combatfx.json'), JSON.stringify(out));
console.log(n, 'effects scanned,', Object.keys(out).length, 'kept,', (JSON.stringify(out).length / 1e3).toFixed(0), 'kB');
const byCls = {}; for (const e of Object.values(out)) byCls[e.cls] = (byCls[e.cls] || 0) + 1; console.log(byCls);
for (const k of ['se_impact_slashing', 'se_general_arrow_01', 'se_arrow01', 'se_weaponswing_sword']) console.log(k, JSON.stringify(out[k] || null).slice(0, 900));
