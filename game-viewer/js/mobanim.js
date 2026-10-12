// The game's own monster animations (data/mobanims/ <- tools/build_mobanims.js, decoded from export/mesh/sfa/*.sfas): idle / walk clips
// of local bone transforms, evaluated at a fractional frame and written straight onto the GLB's bones.  The clip data is in the game's left-handed
// space, the GLBs are mirrored in z, so translations negate z and rotations become (-qx, -qy, qz, qw).
import * as THREE from 'three';

const cache = new Map(), q = new THREE.Quaternion(), v = new THREE.Vector3();
export const CLIPS = ['idle', 'walk'];
const json = u => fetch(u).then(r => r.ok ? r.json() : null).catch(() => null);
let index = null;

/** Fetch a model's clips (null if it has none).  Channels are prepared for evalChannel(). */
export function loadClips(file) {
  if (!cache.has(file)) cache.set(file, (async () => {
    const hash = ((index ||= json('/data/mobanims/index.json')), (await index)?.[file]);
    return hash ? prep(await json(`/data/mobanims/${hash}.json`)) : null;
  })());
  return cache.get(file);
}
function prep(raw) {
  if (!raw) return null;
  const out = {};
  for (const k of CLIPS) {
    const c = raw[k]; if (!c) continue;
    const bones = {};
    for (const [name, ch] of Object.entries(c.b)) bones[name] = ch.map(x => {
      if (typeof x === 'number' || Array.isArray(x)) return x;
      let s = 0; return { l: x.l, c: x.c, hi: x.l.map(n => (s += n)) };   // hi: last frame of each piece
    });
    out[k] = { name: c.n, nk: c.nk, dt: raw.dt, period: (c.nk - 1) * raw.dt, bones };   // the last frame equals the first
  }
  return out;
}

/** One channel at (fractional) frame f.  number: constant; array: one sample per frame; { c, hi }: piecewise polynomial in global time f*dt. */
export function evalChannel(ch, f, dt) {
  if (typeof ch === 'number') return ch;
  if (Array.isArray(ch)) { const i = Math.min(f | 0, ch.length - 1), j = Math.min(i + 1, ch.length - 1), k = f - i; return ch[i] + (ch[j] - ch[i]) * k; }
  let p = 0; while (p < ch.hi.length - 1 && f > ch.hi[p]) p++;
  const c = ch.c[p], t = f * dt; let r = 0; for (let i = c.length - 1; i >= 0; i--) r = r * t + c[i];
  return r;
}

/** Bone tracks of every clip for one clone of the model: [{ bone, ch }].  Bones not named in a clip keep their rest pose. */
export function bindClips(clone, clips) {
  const byName = {}; clone.traverse(n => { if (n.isBone) byName[n.name] = n; });
  const tracks = {};
  for (const k of CLIPS) if (clips[k]) tracks[k] = Object.entries(clips[k].bones).filter(([n]) => byName[n]).map(([n, ch]) => ({ bone: byName[n], ch }));
  return { clips, tracks };
}

/** Pose the bones at frame f of a clip; w < 1 blends over whatever pose the bones hold now. */
export function applyClip(part, name, f, w = 1) {
  const clip = part.clips[name], tr = part.tracks[name]; if (!clip || !tr) return;
  const dt = clip.dt;
  for (const { bone, ch } of tr) {
    v.set(evalChannel(ch[0], f, dt), evalChannel(ch[1], f, dt), -evalChannel(ch[2], f, dt));
    q.set(-evalChannel(ch[3], f, dt), -evalChannel(ch[4], f, dt), evalChannel(ch[5], f, dt), evalChannel(ch[6], f, dt)).normalize();
    if (w >= 1) { bone.position.copy(v); bone.quaternion.copy(q); } else { bone.position.lerp(v, w); bone.quaternion.slerp(q, w); }
  }
}

/** Ground speed (model units per second) the walk clip was authored for: the feet travel back by the length of their stride while they are on the ground
 *  (about 60% of a cycle), so speed ~ stride / (0.6 * period).  Measured on the posed skeleton; null if the model has no leg bones. */
export function walkSpeed(clone, part) {
  const clip = part.clips.walk; if (!clip) return null;
  const feet = []; clone.traverse(n => { if (n.isBone && /(Leg|Arm)Ankle$/.test(n.name)) feet.push(n); }); if (!feet.length) return null;
  const lo = feet.map(() => Infinity), hi = feet.map(() => -Infinity), p = new THREE.Vector3();
  clone.updateMatrixWorld(true);
  const inv = new THREE.Matrix4().copy(clone.matrixWorld).invert();
  for (let f = 0; f < clip.nk; f++) {
    applyClip(part, 'walk', f); clone.updateMatrixWorld(true);
    feet.forEach((b, i) => { p.setFromMatrixPosition(b.matrixWorld).applyMatrix4(inv); lo[i] = Math.min(lo[i], p.z); hi[i] = Math.max(hi[i], p.z); });
  }
  const stride = Math.max(...feet.map((_, i) => hi[i] - lo[i]));
  return stride > 1 ? stride / (0.6 * clip.period) : null;
}

/** Which way the model's front points along its own z axis: 1 = +Z, -1 = -Z (the meshes are mixed, ~55% / 45%).  The walk clip knows: a foot on the ground travels
 *  backwards relative to the body.  Without a clip (or too little foot travel) vote on rest-pose cues that sit ahead of their joint: toes, fingers, jaw, head.
 *  Quadrupeds with static feet get it wrong sometimes, so FACE_FIX overrides (checked by eye).  Call on an unposed clone. */
const FACE_FIX = { wolf: 1, akathar: 1, hivekin_queen: 1, gorra_slave: 1, evil_eye: 1 };
export function facing(clone, part, file) {
  if (FACE_FIX[file]) return FACE_FIX[file];
  const feet = [], by = {}; clone.traverse(n => { if (!n.isBone) return; by[n.name.replace(/^[a-z]+_f/, '')] ??= n; if (/(Leg|Arm)Ankle$/.test(n.name)) feet.push(n); });
  clone.updateMatrixWorld(true);
  const z = n => v.setFromMatrixPosition(by[n].matrixWorld).z, ahead = (a, b) => by[a] && by[b] ? z(a) - z(b) : 0;
  const digit = (k, p) => Object.keys(by).find(n => n.startsWith(k + 'Digit') || n.startsWith(p));
  const toe = digit('LLegAnkle'), finger = Object.keys(by).find(n => n.startsWith('LArmPalmDigit'));
  const vote = Math.sign(toe ? ahead(toe, 'LLegAnkle') : 0) * 2 + Math.sign(finger ? ahead(finger, 'LArmPalm') : 0) * 2 + Math.sign(ahead('Jaw', 'Head')) + Math.sign(ahead('Head', 'Pelvis'));
  const clip = part?.clips.walk; let stance = 0;
  if (clip && feet.length) {
    for (const ank of feet) {
      const ys = [], zs = [];
      for (let f = 0; f < clip.nk; f++) { applyClip(part, 'walk', f); clone.updateMatrixWorld(true); v.setFromMatrixPosition(ank.matrixWorld); ys.push(v.y); zs.push(v.z); }
      const thr = [...ys].sort((a, b) => a - b)[Math.floor(ys.length * 0.4)]; let s = 0, n = 0;
      for (let f = 0; f < ys.length - 1; f++) if (ys[f] <= thr && ys[f + 1] <= thr) { s += zs[f + 1] - zs[f]; n++; }
      if (n) stance += s / n;
    }
  }
  if (Math.abs(stance) > 0.05) return stance > 0 ? -1 : 1;   // the stance foot moves towards the back
  return vote > 0 ? 1 : -1;
}
