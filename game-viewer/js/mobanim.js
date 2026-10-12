// The game's own monster animations (data/mobanims/ <- tools/build_mobanims.js, decoded from export/mesh/sfa/*.sfas): idle / walk clips
// of local bone transforms, evaluated at a fractional frame and written straight onto the GLB's bones.  The clip data is in the game's left-handed
// space, the GLBs are mirrored in z, so translations negate z and rotations become (-qx, -qy, qz, qw).
import * as THREE from 'three';

const cache = new Map(), q = new THREE.Quaternion(), v = new THREE.Vector3();
const Y180 = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), Math.PI);
export const CLIPS = ['idle', 'walk'];
const json = u => fetch(u).then(r => r.ok ? r.json() : null).catch(() => null);
let index = null, pindex = null;
const clipNames = raw => Object.keys(raw).filter(k => k !== 'dt');

/** Fetch a model's clips (null if it has none).  Channels are prepared for evalChannel(). */
export function loadClips(file) {
  if (!cache.has(file)) cache.set(file, (async () => {
    const hash = ((index ||= json('/data/mobanims/index.json')), (await index)?.[file]);
    return hash ? prep(await json(`/data/mobanims/${hash}.json`)) : null;
  })());
  return cache.get(file);
}
/** Player clips (data/playeranims/ <- tools/build_playeranims.js), by '<race>_<m|f>' (race: hum, alfar, dwarf, mahirim, mirdain, ork): every clip of the file. */
export function loadPlayerClips(key) {
  const ck = 'player:' + key;
  if (!cache.has(ck)) cache.set(ck, (async () => {
    const hash = ((pindex ||= json('/data/playeranims/index.json')), (await pindex)?.[key]);
    return hash ? prep(await json(`/data/playeranims/${hash}.json`)) : null;
  })());
  return cache.get(ck);
}
function prep(raw) {
  if (!raw) return null;
  const out = {};
  for (const k of clipNames(raw)) {
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
export function bindClips(clone, clips, flip = false) {
  const byName = {}; clone.traverse(n => { if (n.isBone) byName[n.name] = n; });
  const tracks = {};
  for (const k of Object.keys(clips)) tracks[k] = Object.entries(clips[k].bones).filter(([n]) => byName[n]).map(([n, ch]) => ({ bone: byName[n], ch, root: !byName[n].parent?.isBone }));
  return { clips, tracks, flip };
}

/** Many models (about 100, the ones whose skeleton faces the other way) have their skeleton turned 180 degrees about Y relative to the mesh: each vertex sits at
 *  the half-turned position of the bone it is skinned to.  The rest pose hides it (skin matrices cancel), but any bone rotation swings the vertices about a
 *  pivot on the far side of the body (arms like wings).  Compare the fit both ways on the rest pose. */
export function needsFlip(sc) {
  sc.updateMatrixWorld(true);
  let plain = 0, turned = 0, n = 0; const p = new THREE.Vector3(), b = new THREE.Vector3();
  sc.traverse(m => {
    if (!m.isSkinnedMesh) return;
    const si = m.geometry.attributes.skinIndex, sw = m.geometry.attributes.skinWeight, pos = m.geometry.attributes.position;
    for (let i = 0; i < pos.count; i++) {
      if (sw.getComponent(i, 0) < 0.99) continue;
      p.fromBufferAttribute(pos, i).applyMatrix4(m.matrixWorld); b.setFromMatrixPosition(m.skeleton.bones[si.getComponent(i, 0)].matrixWorld);
      plain += p.distanceTo(b); turned += p.set(-p.x, p.y, -p.z).distanceTo(b); n++;
    }
  });
  return n > 0 && turned < plain * 0.8;
}
/** Turn the skeleton roots half a turn about Y and rebuild the inverse bind matrices from that pose (the mesh itself stays). */
export function flipSkeleton(sc) {
  sc.traverse(b => { if (b.isBone && !b.parent?.isBone) { b.position.set(-b.position.x, b.position.y, -b.position.z); b.quaternion.premultiply(Y180); } });
  sc.updateMatrixWorld(true);
  sc.traverse(m => { if (m.isSkinnedMesh) m.skeleton.calculateInverses(); });
}

/** Pose the bones at frame f of a clip; w < 1 blends over whatever pose the bones hold now. */
export function applyClip(part, name, f, w = 1) {
  const clip = part.clips[name], tr = part.tracks[name]; if (!clip || !tr) return;
  const dt = clip.dt;
  const up = clip.up;
  for (const { bone, ch, root } of tr) {
    v.set(evalChannel(ch[0], f, dt), evalChannel(ch[1], f, dt), -evalChannel(ch[2], f, dt));
    q.set(-evalChannel(ch[3], f, dt), -evalChannel(ch[4], f, dt), evalChannel(ch[5], f, dt), evalChannel(ch[6], f, dt)).normalize();
    if (root && part.flip) { v.set(-v.x, v.y, -v.z); q.premultiply(Y180); }   // see needsFlip
    if (root && up) { v.sub(up.pivot).applyQuaternion(up.q).add(up.pivot); q.premultiply(up.q); }   // see uprightClips
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

/** Which way the model's front points along its own z axis (after flipSkeleton nearly every model faces +Z, so that is the default).  A walk clip that moves the
 *  stance foot clearly the other way overrides it: a foot on the ground travels backwards relative to the body.  Call on an unposed clone. */
export function facing(clone, part) {
  const clip = part?.clips.walk, feet = []; clone.traverse(n => { if (n.isBone && /(Leg|Arm)Ankle$/.test(n.name)) feet.push(n); });
  if (!clip || !feet.length) return 1;
  let stance = 0;
  for (const ank of feet) {
    const ys = [], zs = [];
    for (let f = 0; f < clip.nk; f++) { applyClip(part, 'walk', f); clone.updateMatrixWorld(true); v.setFromMatrixPosition(ank.matrixWorld); ys.push(v.y); zs.push(v.z); }
    const thr = [...ys].sort((a, b) => a - b)[Math.floor(ys.length * 0.4)]; let s = 0, n = 0;
    for (let f = 0; f < ys.length - 1; f++) if (ys[f] <= thr && ys[f + 1] <= thr) { s += zs[f + 1] - zs[f]; n++; }
    if (n) stance += s / n;
  }
  return stance > 0.5 ? -1 : 1;
}

/** Some clips hold the whole body rolled sideways by 20-35 degrees in every frame (the gnoll / hivekin / lizardman walks).  Measure the mean roll of each clip once
 *  (head against the middle of the feet, about the forward z axis) and let applyClip turn the skeleton root back about the feet.  Only for clear bipeds:
 *  quadrupeds have no meaningful head-over-feet axis.  Call on an unposed clone of each model; clips are shared, so it runs once per file. */
export function uprightClips(clone, part) {
  const by = {}; clone.traverse(n => { if (n.isBone) by[n.name.replace(/^[a-z]+_f/, '')] ??= n; });
  const ok = by.Head && by.LLegAnkle && by.RLegAnkle;
  const inv = new THREE.Matrix4(), a = new THREE.Vector3(), b = new THREE.Vector3(), h = new THREE.Vector3();
  for (const k of CLIPS) {
    const clip = part.clips[k]; if (!clip || clip.up !== undefined) continue;
    clip.up = null; if (!ok) continue;
    clone.updateMatrixWorld(true); inv.copy(clone.matrixWorld).invert();
    let roll = 0, n = 0, fx = 0; const pivot = new THREE.Vector3();
    for (let f = 0; f < clip.nk; f += 2) {
      applyClip(part, k, f); clone.updateMatrixWorld(true);
      a.setFromMatrixPosition(by.LLegAnkle.matrixWorld).applyMatrix4(inv); b.setFromMatrixPosition(by.RLegAnkle.matrixWorld).applyMatrix4(inv); a.add(b).multiplyScalar(0.5);
      h.setFromMatrixPosition(by.Head.matrixWorld).applyMatrix4(inv).sub(a);
      roll += Math.atan2(h.x, h.y); fx += Math.abs(h.z) / Math.max(1, h.y); pivot.add(a); n++;
    }
    roll /= n; pivot.multiplyScalar(1 / n);
    if (Math.abs(roll) < 0.05 || Math.abs(roll) > 0.8 || fx / n > 0.6) continue;
    clip.up = { q: new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), roll), pivot };
  }
}
