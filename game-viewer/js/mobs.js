// Server-owned mobs (see ../../game-server/src/mobs.rs and proto.rs OP_MOBS): the server streams the ones inside our interest radius at ~5 Hz;
// we interpolate them, drop them onto the terrain and show the game's own monster model (data/mobmodels.json <- tools/build_mobmodels.js,
// converted/animated/<name>.glb, skinned, textures embedded) in its bind pose with a small walking sway. No skeletal animation is ported.
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { AVATAR_SCALE, cloneSkinned } from './avatars.js';

export const OP_MOBS = 3;
const FLAG_SNAP = 4;
const INTERP = 0.4, GONE = 1.5;                    // snapshots are 0.2 s apart; a mob unseen this long left the interest radius
const MODEL_NEAR = 300, TAG_NEAR = 45, SHADOW_NEAR = 70, MAX_BUILD = 3;
const lerpAngle = (a, b, t) => a + Math.atan2(Math.sin(b - a), Math.cos(b - a)) * t;
const DIFF_COLOR = { 'Very Easy': '#9fd68a', Easy: '#9fd68a', Medium: '#f1dfae', Hard: '#f0b070', 'Very Hard': '#e88a6a', Boss: '#e0607a', Champion: '#d070e0' };

export class MobWorld {
  constructor({ scene, camera, world }) {
    Object.assign(this, { scene, camera, world });
    this.gltf = new GLTFLoader(); this.types = new Map(); this.mobs = new Map(); this.tpl = new Map(); this.tags = new Map();
    this.models = null; this.building = 0; this.tmp = {};
  }

  clear() { for (const m of this.mobs.values()) this.release(m); this.mobs.clear(); this.types.clear(); }
  setTypes(list) { this.types = new Map(list.map(t => [t.id, t])); this.models ||= fetch('/data/mobmodels.json').then(r => r.json()).catch(() => ({})); }

  onFrame(buf) {
    const v = new DataView(buf), n = v.getUint16(5, true), now = performance.now() / 1000;
    for (let i = 0, o = 7; i < n; i++, o += 22) {
      const id = v.getUint16(o, true);
      let m = this.mobs.get(id); if (!m) this.mobs.set(id, m = { id, kind: v.getUint16(o + 2, true), states: [], seen: 0, obj: null, phase: Math.random() * 6.28, gy: null, gyT: 0 });
      m.states.push({ t: now, x: v.getFloat32(o + 4, true), y: v.getFloat32(o + 8, true), z: v.getFloat32(o + 12, true), yaw: v.getFloat32(o + 16, true), speed: v.getUint8(o + 20) / 10, flags: v.getUint8(o + 21) });
      if (m.states.length > 4) m.states.shift(); m.seen = now;
    }
  }

  // one template per model file; every mob clones it (shared geometry / materials)
  template(file) {
    if (!this.tpl.has(file)) this.tpl.set(file, this.gltf.loadAsync(`/converted/animated/${file}.glb`).then(g => {
      g.scene.traverse(n => { if (n.isMesh) { n.frustumCulled = false; for (const mt of [].concat(n.material)) mt.side = THREE.DoubleSide; } });
      return g.scene;
    }, () => null));
    return this.tpl.get(file);
  }
  async build(m) {
    m.building = true; this.building++;
    try {
      const files = (await this.models)[m.kind]?.models || [], root = new THREE.Group();
      for (const f of files) { const t = await this.template(f); if (t) root.add(cloneSkinned(t)); }
      root.scale.setScalar(AVATAR_SCALE); const holder = new THREE.Group(); holder.add(root);
      if (this.mobs.get(m.id) === m) { m.obj = holder; m.model = root; } else m.dead = true;
    } catch (e) { console.warn('mob model', m.kind, e); } finally { m.building = false; m.tried = true; this.building--; }
  }
  tagFor(kind) {
    if (!this.tags.has(kind)) {
      const t = this.types.get(kind), name = t?.name || '?', f = '600 28px Palatino, Georgia, serif', c = document.createElement('canvas'), x = c.getContext('2d');
      x.font = f; const w = Math.ceil(x.measureText(name).width) + 24; c.width = w; c.height = 52; x.font = f;
      x.textAlign = 'center'; x.textBaseline = 'middle'; x.fillStyle = '#0007'; x.beginPath(); x.roundRect(0, 5, w, 42, 12); x.fill();
      x.fillStyle = DIFF_COLOR[t?.difficulty] || '#f1dfae'; x.fillText(name, w / 2, 27);
      const tex = new THREE.CanvasTexture(c); tex.colorSpace = THREE.SRGBColorSpace;
      this.tags.set(kind, { mat: new THREE.SpriteMaterial({ map: tex, transparent: true, depthWrite: false, fog: false }), aspect: w / 52 });
    }
    return this.tags.get(kind);
  }
  release(m) { if (m.obj) this.scene.remove(m.obj); if (m.tag) this.scene.remove(m.tag); m.obj = m.tag = m.model = null; m.shown = false; m.tried = false; }

  sample(m, rt, out) {
    const S = m.states; let a = S[0], b = S[S.length - 1];
    for (let i = S.length - 1; i > 0; i--) if (S[i - 1].t <= rt) { a = S[i - 1]; b = S[i]; break; }
    const k = b.t > a.t ? Math.min(1, Math.max(0, (rt - a.t) / (b.t - a.t))) : 1;
    out.x = a.x + (b.x - a.x) * k; out.y = a.y + (b.y - a.y) * k; out.z = a.z + (b.z - a.z) * k; out.yaw = lerpAngle(a.yaw, b.yaw, k); out.speed = b.speed; out.flags = b.flags;
  }

  update(dt, now) {
    const cam = this.camera.position, rt = now - INTERP, st = this.tmp, fovK = 2 * Math.tan(THREE.MathUtils.degToRad(this.camera.fov / 2)) / innerHeight;
    for (const m of this.mobs.values()) {
      if (now - m.seen > GONE) { this.release(m); this.mobs.delete(m.id); continue; }
      const last = m.states[m.states.length - 1], dx = last.x - cam.x, dz = last.z - cam.z, d2 = dx * dx + dz * dz;
      if (d2 > MODEL_NEAR * MODEL_NEAR) { if (m.obj && m.shown) { this.scene.remove(m.obj, m.tag); m.shown = false; } continue; }
      if (!m.obj) { if (!m.building && !m.tried && this.models && this.building < MAX_BUILD) this.build(m); continue; }
      this.sample(m, rt, st);
      if (st.flags & FLAG_SNAP) {
        if (now - m.gyT > 0.1 || m.gy == null) { const g = this.world.groundHeight(st.x, st.z); if (g != null && g > -1e8) m.gy = g; m.gyT = now; }
        st.y = m.gy ?? st.y;
      }
      if (!m.shown) { this.scene.add(m.obj); m.shown = true; }
      const dist = Math.sqrt(d2);
      if (st.speed > 0.1) m.phase += dt * 5; const walk = st.speed > 0.1 ? 1 : 0;
      m.obj.position.set(st.x, st.y + Math.abs(Math.sin(m.phase)) * 0.05 * walk, st.z); m.obj.rotation.y = st.yaw; m.model.rotation.z = Math.sin(m.phase) * 0.03 * walk;
      const shadow = dist < SHADOW_NEAR; if (shadow !== m.cast) { m.cast = shadow; m.obj.traverse(n => { if (n.isMesh) n.castShadow = shadow; }); }
      if (dist < TAG_NEAR) {
        if (!m.tag) { const t = this.tagFor(m.kind); m.tag = new THREE.Sprite(t.mat); m.tag.renderOrder = 20; m.tag.center.set(0.5, 0); m.tag.userData.aspect = t.aspect; }
        if (!m.tag.parent) this.scene.add(m.tag);
        if (m.height == null) m.height = new THREE.Box3().setFromObject(m.obj).getSize(new THREE.Vector3()).y;
        const th = 20 * fovK * dist; m.tag.position.set(st.x, st.y + m.height + 0.2, st.z); m.tag.scale.set(th * m.tag.userData.aspect, th, 1);
      } else if (m.tag?.parent) this.scene.remove(m.tag);
    }
  }

  /** Radar points: x/z of every streamed mob. */
  each(cb) { for (const m of this.mobs.values()) { const s = m.states[m.states.length - 1]; cb(s.x, s.z); } }
}
