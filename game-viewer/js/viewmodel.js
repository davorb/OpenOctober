// First-person weapon view: its own scene + camera (short near plane, so the weapon never clips into walls) that js/post.js draws over the world after
// clearing depth.  The weapon is oriented from its own geometry (long axis = blade / shaft / limb, heavy end away from the grip) and moved procedurally: the game
// has no first-person melee clips (its melee skills only target the third-person body), and its fp_hands meshes (converted/animated/<race>_<g>_hands.glb with the
// <race>_<g>_hands.sfas bow / magic / grenade clips) deform into sheets under those clips here, so no hands are drawn.  Not ported.
import * as THREE from 'three';

const V = (x, y, z) => new THREE.Vector3(x, y, z), E = new THREE.Euler(0, 0, 0, 'YXZ');
const smooth = t => t * t * (3 - 2 * t);

// swing keyframes for the grip in camera space: [t (0..1 of the attack), x, y, z, pitch, yaw, roll]  (angles are added to the resting orientation)
const R = [0, 0.34, -0.32, -0.62, 0, 0, 0], REST = { melee: R, staff: [0, 0.30, -0.36, -0.55, 0, 0, 0], bow: [0, -0.20, -0.14, -0.60, 0, 0, 0], thrown: [0, 0.30, -0.30, -0.55, 0, 0, 0] };
const back = (r, k) => [1, ...r.slice(1)];
const SWINGS = {
  slash:    [R, [0.28, 0.52, -0.10, -0.46, 0.55, -0.55, -0.9], [0.50, -0.34, -0.28, -0.78, -0.55, 0.85, 0.7], [0.72, -0.18, -0.30, -0.66, -0.2, 0.45, 0.3], back(R)],
  backhand: [R, [0.28, -0.16, -0.20, -0.50, 0.25, 0.6, 0.9], [0.50, 0.62, -0.30, -0.78, -0.45, -0.85, -0.6], [0.72, 0.5, -0.30, -0.66, -0.2, -0.4, -0.3], back(R)],
  chop:     [R, [0.30, 0.30, 0.10, -0.38, 1.0, 0, -0.1], [0.50, 0.12, -0.46, -0.78, -0.95, 0, 0.1], [0.72, 0.2, -0.40, -0.66, -0.4, 0, 0], back(R)],
  stab:     [R, [0.30, 0.38, -0.22, -0.34, 0.55, 0.1, 0], [0.48, 0.10, -0.24, -1.00, -0.5, 0, 0], [0.70, 0.18, -0.28, -0.80, -0.2, 0, 0], back(R)],
  cast:     [REST.staff, [0.35, 0.26, -0.20, -0.45, 0.7, 0, 0.2], [0.55, 0.12, -0.18, -0.85, -0.3, 0, 0], [0.8, 0.2, -0.28, -0.7, -0.1, 0, 0], back(REST.staff)],
  throw:    [REST.thrown, [0.30, 0.42, 0.0, -0.30, 0.9, 0, 0], [0.50, 0.05, -0.30, -0.9, -0.8, 0, 0], [0.75, 0.2, -0.3, -0.7, -0.2, 0, 0], back(REST.thrown)],
  shoot:    [[0, -0.20, -0.10, -0.44, 0.1, 0.15, 0], [0.25, -0.20, -0.12, -0.62, -0.1, 0, 0], back(REST.bow)],   // recoil after the release
};
// holds (right mouse = guard / bow draw): pose reached by easing in, kept while held
const HOLDS = { block: [0, 0.04, -0.06, -0.55, 0.6, 0.15, -0.2], draw: [0, -0.20, -0.10, -0.44, 0.1, 0.15, 0] };
const HIT_AT = 0.45;                                             // the skills' Synchronized_StateChange: the blow lands at 45% of the animation
// resting orientation per weapon class: [direction of the long axis, direction of the thinnest axis (the flat of a blade / the belly of a bow)]
const ORIENT = { melee: [V(0.16, 0.74, -0.65), V(1, 0, 0)], staff: [V(0.1, 0.85, -0.5), V(1, 0, 0)], bow: [V(0, 1, -0.05), V(0, 0, 1)], thrown: [V(0.1, 0.8, -0.6), V(1, 0, 0)] };
const MAXLEN = { melee: 0.72, staff: 1.0, bow: 0.8, thrown: 0.3 };

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
    const kind = { bow: 'bow', staff: 'staff', thrown: 'thrown' }[w.group] || 'melee', [dirT, flatT] = ORIENT[kind];
    const o = src.clone(true); o.traverse(n => { if (n.isMesh) n.frustumCulled = false; });
    const b = new THREE.Box3().setFromObject(o), s = b.getSize(V(0, 0, 0)), ax = [0, 1, 2].sort((a, c) => s.getComponent(c) - s.getComponent(a));
    const dir = V(0, 0, 0); dir.setComponent(ax[0], kind === 'bow' ? 1 : Math.abs(b.max.getComponent(ax[0])) > Math.abs(b.min.getComponent(ax[0])) ? 1 : -1);
    const flat = V(0, 0, 0); flat.setComponent(ax[2], 1);
    const target = dirT.clone().normalize(), q1 = new THREE.Quaternion().setFromUnitVectors(dir, target);
    const flatNow = flat.clone().applyQuaternion(q1), proj = flatT.clone().sub(target.clone().multiplyScalar(flatT.dot(target))).normalize();
    const f = flatNow.clone().sub(target.clone().multiplyScalar(flatNow.dot(target))).normalize(), roll = Math.atan2(target.dot(f.clone().cross(proj)), f.dot(proj));
    this.q0.setFromAxisAngle(target, Math.atan2(Math.sin(roll), Math.cos(roll)) % Math.PI).multiply(q1);   // flat up to a half turn: either face may point to the camera
    // grip: melee weapons are held at the origin of their model; bows are centred (the model origin is the grip too, but limbs run both ways)
    const len = s.getComponent(ax[0]), scale = Math.min(1, MAXLEN[kind] / Math.max(0.3, len)); o.scale.setScalar(scale);
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
