// First-person weapon view: its own scene + camera (short near plane, so the weapon never clips into walls) that js/post.js draws over the world after
// clearing depth.  The weapon is oriented from its own geometry (long axis = blade / shaft / limb, heavy end away from the grip) and moved procedurally: the game
// has no first-person melee clips (its melee skills only target the third-person body), and its fp_hands meshes (converted/animated/<race>_<g>_hands.glb with the
// <race>_<g>_hands.sfas bow / magic / grenade clips) deform into sheets under those clips here, so no hands are drawn.  Not ported.
import * as THREE from 'three';

const V = (x, y, z) => new THREE.Vector3(x, y, z), E = new THREE.Euler(0, 0, 0, 'YXZ');
const smooth = t => t * t * (3 - 2 * t);

// swing keyframes for the grip in camera space: [t (0..1 of the attack), x, y, z, pitch, yaw, roll]  (angles are added to the resting orientation)
const R = [0, 0.34, -0.32, -0.62, 0, 0, 0], REST = { melee: R, staff: [0, 0.30, -0.36, -0.55, 0, 0, 0], bow: [0, -0.20, -0.14, -0.60, 0, 0.45, 0], thrown: [0, 0.30, -0.30, -0.55, 0, 0, 0] };
const back = (r, k) => [1, ...r.slice(1)];
const SWINGS = {
  slash:    [R, [0.28, 0.52, -0.10, -0.46, 0.55, -0.55, -0.9], [0.50, -0.34, -0.28, -0.78, -0.55, 0.85, 0.7], [0.72, -0.18, -0.30, -0.66, -0.2, 0.45, 0.3], back(R)],
  backhand: [R, [0.28, -0.16, -0.20, -0.50, 0.25, 0.6, 0.9], [0.50, 0.62, -0.30, -0.78, -0.45, -0.85, -0.6], [0.72, 0.5, -0.30, -0.66, -0.2, -0.4, -0.3], back(R)],
  chop:     [R, [0.30, 0.30, 0.10, -0.38, 1.0, 0, -0.1], [0.50, 0.12, -0.46, -0.78, -0.95, 0, 0.1], [0.72, 0.2, -0.40, -0.66, -0.4, 0, 0], back(R)],
  stab:     [R, [0.30, 0.38, -0.22, -0.34, 0.55, 0.1, 0], [0.48, 0.10, -0.24, -1.00, -0.5, 0, 0], [0.70, 0.18, -0.28, -0.80, -0.2, 0, 0], back(R)],
  cast:     [REST.staff, [0.35, 0.26, -0.20, -0.45, 0.7, 0, 0.2], [0.55, 0.12, -0.18, -0.85, -0.3, 0, 0], [0.8, 0.2, -0.28, -0.7, -0.1, 0, 0], back(REST.staff)],
  throw:    [REST.thrown, [0.30, 0.42, 0.0, -0.30, 0.9, 0, 0], [0.50, 0.05, -0.30, -0.9, -0.8, 0, 0], [0.75, 0.2, -0.3, -0.7, -0.2, 0, 0], back(REST.thrown)],
  shoot:    [[0, -0.20, -0.10, -0.44, 0.1, 0.55, 0], [0.25, -0.20, -0.12, -0.62, -0.1, 0.4, 0], back(REST.bow)],   // recoil after the release
};
// holds (right mouse = guard / bow draw): pose reached by easing in, kept while held
const HOLDS = { block: [0, 0.04, -0.06, -0.55, 0.6, 0.15, -0.2], draw: [0, -0.20, -0.10, -0.44, 0.1, 0.55, 0] };
const HIT_AT = 0.45;                                             // the skills' Synchronized_StateChange: the blow lands at 45% of the animation
// resting orientation per weapon class: [where the business end points, where the edge of the blade / the belly of the bow points] (camera space, -z is away from the player)
const ORIENT = { melee: [V(0.16, 0.74, -0.65), V(-0.35, 0, -1)], staff: [V(0.1, 0.85, -0.5), V(-0.35, 0, -1)], bow: [V(0, 1, -0.05), V(0, 0, 1)], thrown: [V(0.1, 0.8, -0.6), V(-0.35, 0, -1)] };
const MAXLEN = { melee: 0.72, staff: 1.0, bow: 0.8, thrown: 0.3 };

/** Geometry of a weapon model: `d` = unit vector (model space) along the long axis towards the business end (the end with the wider cross-section: blade, axe head, club head, staff
 *  top; for a bow either end), `e` = unit vector across it where the edge sticks out (an axe's blade, a sword's wider side; for a bow the side the limbs curve towards,
 *  the archer's side), `size` = bounding box, `len` = length of the long axis.  The model origin is the grip. */
export function analyze(o, kind) {
  o.updateMatrixWorld(true); const pts = [], p = V(0, 0, 0);
  o.traverse(n => { if (!n.isMesh) return; const pa = n.geometry.attributes.position, step = Math.max(1, Math.floor(pa.count / 3000)); for (let i = 0; i < pa.count; i += step) pts.push(p.fromBufferAttribute(pa, i).applyMatrix4(n.matrixWorld).toArray()); });
  const lo = [1e9, 1e9, 1e9], hi = [-1e9, -1e9, -1e9]; for (const q of pts) for (let k = 0; k < 3; k++) { lo[k] = Math.min(lo[k], q[k]); hi[k] = Math.max(hi[k], q[k]); }
  const size = V(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]), ax = [0, 1, 2].sort((a, c) => size.getComponent(c) - size.getComponent(a)), [L, M, T] = ax, len = size.getComponent(L);
  // cross-section (extent over the other two axes) of the two ends of the long axis; the wider end is the business end
  // cross-section (extent over the other two axes) of the two ends of the long axis; the wider end is the business end
  const end = (sgn) => { const a0 = sgn > 0 ? hi[L] - len * 0.25 : lo[L], a1 = sgn > 0 ? hi[L] : lo[L] + len * 0.25; let mn = [1e9, 1e9], mx = [-1e9, -1e9], n = 0; const m = [0, 0];
    for (const q of pts) if (q[L] >= a0 && q[L] <= a1) { n++; m[0] += q[M]; m[1] += q[T]; mn[0] = Math.min(mn[0], q[M]); mx[0] = Math.max(mx[0], q[M]); mn[1] = Math.min(mn[1], q[T]); mx[1] = Math.max(mx[1], q[T]); }
    return { w: (mx[0] - mn[0]) + (mx[1] - mn[1]), m: n ? [m[0] / n, m[1] / n] : [0, 0] }; };
  const A = end(1), B = end(-1), med = k => { const v = pts.map(q => q[k]).sort((a, c) => a - c); return v[v.length >> 1]; }, med2 = [med(M), med(T)];
  let sgn = A.w > B.w * 1.12 ? 1 : B.w > A.w * 1.12 ? -1 : (Math.abs(hi[L]) >= Math.abs(lo[L]) ? 1 : -1);   // similar widths: the end farther from the grip
  // the exported models share a convention: the business end of every melee weapon and staff (the head) is at -z of the grip; a bow runs both ways (limbs)
  if (L === 2) sgn = kind === 'bow' ? 1 : -1;
  // the side axis: where the mass of the business end (a bow: both tips) is displaced from the middle of the weapon - the blade of an axe, the curve of a bow
  const ends = kind === 'bow' ? [A, B] : [sgn > 0 ? A : B], dev = [0, 1].map(i => ends.reduce((s, x) => s + x.m[i], 0) / ends.length - med2[i]), side = Math.abs(dev[0]) >= Math.abs(dev[1]) ? 0 : 1;
  const sym = Math.abs(dev[side]) < 0.03 * len;   // symmetrical (a sword): no side to speak of, show the wide face
  const d = V(0, 0, 0), e = V(0, 0, 0); d.setComponent(L, sgn); e.setComponent(sym || !side ? M : T, sym ? 1 : dev[side] >= 0 ? 1 : -1);
  return { d, e, size, len };
}

export class ViewModel {
  constructor(factory) {
    this.factory = factory; this.visible = true; this.scene = new THREE.Scene();
    this.cam = new THREE.PerspectiveCamera(62, 1, 0.02, 20);
    this.hemi = new THREE.HemisphereLight(0xffffff, 0x887766, 2.4); this.sun = new THREE.DirectionalLight(0xffffff, 2.2); this.sun.position.set(0.5, 1, 0.6);
    this.scene.add(this.hemi, this.sun);
    this.root = new THREE.Group(); this.scene.add(this.root); this.grip = new THREE.Group(); this.root.add(this.grip);
    this.w = null; this.kind = 'melee'; this.atk = null; this.bob = 0; this.seq = 0; this.hold = null; this.onhit = null; this.q0 = new THREE.Quaternion(); this._k = REST.melee.slice();
  }
  resize(aspect) { this.cam.aspect = aspect; this.cam.updateProjectionMatrix(); }
  /** copy the world's light (call every frame): the viewmodel is lit like the scene */
  light(sunLight, hemiLight) { this.sun.color.copy(sunLight.color); this.sun.intensity = sunLight.intensity * 0.6; this.sun.position.copy(sunLight.position).sub(sunLight.target.position).normalize(); this.hemi.color.copy(hemiLight.color); this.hemi.groundColor.copy(hemiLight.groundColor); this.hemi.intensity = hemiLight.intensity * 0.9; }

  /** Show weapon `w` (an entry of data/weapons.json); null = empty hands. */
  async setWeapon(w) {
    const my = ++this.seq; this.w = null; this.atk = null; this.hold = null; this.grip.clear();
    if (!w?.mesh) return;
    const src = await this.factory.static(w.mesh); if (!src || my !== this.seq) return;
    const kind = { bow: 'bow', staff: 'staff', thrown: 'thrown' }[w.group] || 'melee';
    const o = src.clone(true); o.traverse(n => { if (n.isMesh) n.frustumCulled = false; });
    const { d, e, len } = analyze(o, kind), [dirT, edgeT] = [ORIENT[kind][0].clone().normalize(), ORIENT[kind][1].clone()];
    const E2 = edgeT.sub(dirT.clone().multiplyScalar(edgeT.dot(dirT))).normalize(), T2 = dirT.clone().cross(E2);   // target frame: long axis, edge / belly side, third
    const ms = new THREE.Matrix4().makeBasis(d, e, d.clone().cross(e)), mt = new THREE.Matrix4().makeBasis(dirT, E2, T2);
    this.q0.setFromRotationMatrix(mt.multiply(ms.transpose()));
    // grip: melee weapons are held at the origin of their model; bows are centred (the model origin is the grip too, but limbs run both ways)
    const scale = Math.min(1, MAXLEN[kind] / Math.max(0.3, len)); o.scale.setScalar(scale);
    if (kind === 'bow') o.position.set(0, 0, 0);
    this.grip.add(o); this.kind = kind; this.w = w; this._k = REST[kind].slice(); this.pose(this._k);
  }
  /** pose the grip at keyframe values k = [t, x, y, z, pitch, yaw, roll] */
  pose(k) { this.grip.position.set(k[1], k[2], k[3]); this.grip.quaternion.setFromEuler(E.set(k[4], k[5], k[6])).multiply(this.q0); }

  /** Start an attack of the given shape ('slash' | 'backhand' | 'chop' | 'stab' | 'cast' | 'throw' | 'shoot') lasting `dur` seconds; onhit fires when the blow lands. */
  swing(kind, dur) { if (!this.w || this.atk) return false; this.atk = { keys: SWINGS[kind] || SWINGS.slash, t: 0, dur, hit: false }; return true; }
  /** hold a pose ('block' | 'draw', null to let go) */
  setHold(name) { this.hold = name ? HOLDS[name] : null; }
  get busy() { return !!this.atk; }

  update(dt, speed = 0, air = false) {
    this.bob += dt * (3 + speed * 0.9); const amp = Math.min(1, speed / 5) * (air ? 0 : 1);
    this.root.position.set(Math.sin(this.bob) * 0.012 * amp, -Math.abs(Math.cos(this.bob)) * 0.016 * amp + Math.sin(this.bob * 0.37) * 0.003, 0);
    if (!this.w) return;
    let k = REST[this.kind];
    if (this.atk) {
      const a = this.atk; a.t += dt; const p = Math.min(1, a.t / a.dur), ks = a.keys; let i = 0; while (i < ks.length - 2 && p > ks[i + 1][0]) i++;
      const A = ks[i], B = ks[i + 1], f = smooth(Math.min(1, Math.max(0, (p - A[0]) / (B[0] - A[0])))); k = A.map((v, j) => v + (B[j] - v) * f);
      if (!a.hit && p >= HIT_AT) { a.hit = true; this.onhit?.(); }
      if (p >= 1) this.atk = null;
    } else if (this.hold) k = this.hold;
    const cur = this._k, l = this.atk ? 1 : 1 - Math.exp(-dt * 14); for (let j = 1; j < 7; j++) cur[j] += (k[j] - cur[j]) * l; this.pose(cur);
  }
}
