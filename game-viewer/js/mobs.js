// Server-owned mobs (see ../../game-server/src/mobs.rs and proto.rs OP_MOBS): the server streams the ones inside our interest radius at ~5 Hz;
// we interpolate them, drop them onto the terrain and show the game's own monster model (data/mobmodels.json <- tools/build_mobmodels.js,
// converted/animated/<name>.glb, skinned, textures embedded), playing the game's own idle / walk clips (mobanim.js). Models without clips (flying
// and swimming ones) fall back to a procedural walk: nearly every monster skeleton uses the humanoid bone names (quadrupeds' front legs are the 'arms'),
// and gait() swings legs / arms, twists the spine and sways the tail about fixed model axes, the same way avatars.js poses players.
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { AVATAR_SCALE, cloneSkinned } from './avatars.js';
import { Player } from './collision.js';
import { loadClips, bindClips, applyClip, walkSpeed, facing, needsFlip, flipSkeleton, uprightClips } from './mobanim.js';

export const OP_MOBS = 3;
const FLAG_SNAP = 4;
const INTERP = 0.4, GONE = 1.5;                    // snapshots are 0.2 s apart; a mob unseen this long left the interest radius
const MODEL_NEAR = 300, TAG_NEAR = 45, SHADOW_NEAR = 70, MAX_BUILD = 3;
const AX_X = new THREE.Vector3(1, 0, 0), AX_Y = new THREE.Vector3(0, 1, 0), AX_Z = new THREE.Vector3(0, 0, 1), NX = new THREE.Vector3(-1, 0, 0), NZ = new THREE.Vector3(0, 0, -1);
const qa = new THREE.Quaternion(), qb = new THREE.Quaternion();
const GAIT = ['Pelvis', 'LLeg1', 'RLeg1', 'LLeg2', 'RLeg2', 'LArm1', 'RArm1', 'Spine1', 'Spine2', 'Ribcage', 'Neck1', 'Head', 'Tail1', 'Tail2', 'Tail3', 'Tail4', 'Tail5'];
const STEP_UP = 1.4;   // highest step (m) a mob climbs between two floor probes, the player's step height plus slack
const POSE_NEAR = 140, FADE = 0.25;   // clip crossfade (s)
const lerpAngle = (a, b, t) => a + Math.atan2(Math.sin(b - a), Math.cos(b - a)) * t;
const DIFF_COLOR = { 'Very Easy': '#9fd68a', Easy: '#9fd68a', Medium: '#f1dfae', Hard: '#f0b070', 'Very Hard': '#e88a6a', Boss: '#e0607a', Champion: '#d070e0' };

const put = (b, rig, R) => b.quaternion.copy(rig.pqInv).multiply(R).multiply(rig.pq).multiply(rig.q0);
/** Procedural locomotion. Every delta rotates about a fixed model axis (X sideways, Y up, Z forward), converted into the bone's local frame
 *  through its parent's rest rotation, so the bone axis conventions of each skeleton don't matter. s/c: sin/cos of the stride phase, amp: 0 idle .. 1 walking.
 *  The deltas are written for a model whose front is +Z; m.face = -1 (front at -Z) mirrors the X and Z axes. */
export function gait(m, s, c, amp) {
  const B = m.bones, br = Math.sin(m.breath * 1.8), AX_X_ = m.face < 0 ? NX : AX_X, AX_Z_ = m.face < 0 ? NZ : AX_Z;
  const rot = (name, ax, ang) => { const l = B[name]; if (l) for (const { bone, rig } of l) put(bone, rig, qa.setFromAxisAngle(ax, ang)); };
  const rot2 = (name, a1, g1, a2, g2) => { const l = B[name]; if (l) for (const { bone, rig } of l) put(bone, rig, qa.setFromAxisAngle(a1, g1).multiply(qb.setFromAxisAngle(a2, g2))); };
  const leg = 0.38 * amp, arm = 0.34 * amp;
  rot('LLeg1', AX_X_, -leg * s); rot('RLeg1', AX_X_, leg * s);
  rot('LLeg2', AX_X_, Math.max(0, -s) * 0.6 * amp); rot('RLeg2', AX_X_, Math.max(0, s) * 0.6 * amp);   // knees fold as the leg swings forward
  rot('LArm1', AX_X_, arm * s); rot('RArm1', AX_X_, -arm * s);                                            // arms / front legs swing against the same-side leg
  rot2('Pelvis', AX_Y, 0.07 * amp * s, AX_Z_, 0.03 * amp * c);
  rot('Spine1', AX_Y, -0.05 * amp * s); rot('Spine2', AX_Y, -0.05 * amp * s);
  rot('Ribcage', AX_X_, 0.012 * br + 0.02 * amp * Math.abs(c));
  rot('Neck1', AX_Y, 0.05 * amp * s); rot('Head', AX_Y, 0.04 * amp * s);
  const t = m.breath * 1.3;
  ['Tail1', 'Tail2', 'Tail3', 'Tail4', 'Tail5'].forEach((n, i) => rot(n, AX_Y, (0.12 + 0.06 * i) * (0.4 + amp) * Math.sin(t * (1 + amp) - i * 0.7 + (amp ? m.phase * 0.5 : 0))));
}

/** The converted materials are alpha-masked, but many textures carry an empty alpha channel (not real cut-outs): the mask would discard the
 *  whole mesh. If the texture's alpha is (almost) all transparent, drop the alpha test; genuine cut-outs keep it. */
const probe = document.createElement('canvas'); probe.width = probe.height = 32; const pc = probe.getContext('2d', { willReadFrequently: true });
function fixAlpha(mt) {
  if (!mt.alphaTest || !mt.map?.image) return;
  try {
    pc.clearRect(0, 0, 32, 32); pc.drawImage(mt.map.image, 0, 0, 32, 32);
    const px = pc.getImageData(0, 0, 32, 32).data; let clear = 0; for (let i = 3; i < px.length; i += 4) if (px[i] < 128) clear++;
    if (clear / 1024 > 0.9) { mt.alphaTest = 0; mt.needsUpdate = true; }
  } catch { mt.alphaTest = 0; }
}

/** Loaded GLB scene -> template: no frustum culling (bind-pose bounds don't follow the pose), double-sided, and each gait bone's rest rotations. */
export function prepareTemplate(sc) {
  sc.updateMatrixWorld(true); sc.userData.rig = {};
  if (needsFlip(sc)) { flipSkeleton(sc); sc.userData.flip = true; }
  sc.traverse(n => {
    if (n.isMesh) { n.frustumCulled = false; for (const mt of [].concat(n.material)) { mt.side = THREE.DoubleSide; fixAlpha(mt); } }
    if (!n.isBone) return;
    const k = n.name.replace(/^[a-z]+_f/, ''); if (!GAIT.includes(k)) return;
    const pq = new THREE.Quaternion(); n.parent.getWorldQuaternion(pq);
    sc.userData.rig[k] = { q0: n.quaternion.clone(), pq, pqInv: pq.clone().invert() };
  });
  return sc;
}
export function collectBones(clone, tpl, bones) {
  clone.traverse(n => { if (!n.isBone) return; const k = n.name.replace(/^[a-z]+_f/, ''), r = tpl.userData.rig[k]; if (r) (bones[k] ||= []).push({ bone: n, rig: r }); });
}

export class MobWorld {
  constructor({ scene, camera, world }) {
    Object.assign(this, { scene, camera, world });
    this.probe = new Player(world); this.gltf = new GLTFLoader(); this.types = new Map(); this.mobs = new Map(); this.tpl = new Map(); this.tags = new Map();
    this.models = null; this.building = 0; this.tmp = {}; this.stride = new Map(); this.face = new Map();
  }

  clear() { for (const m of this.mobs.values()) this.release(m); this.mobs.clear(); this.types.clear(); }
  setTypes(list) { this.types = new Map(list.map(t => [t.id, t])); this.models ||= fetch('/data/mobmodels.json').then(r => r.json()).catch(() => ({})); }

  onFrame(buf) {
    const v = new DataView(buf), n = v.getUint16(5, true), now = performance.now() / 1000;
    for (let i = 0, o = 7; i < n; i++, o += 22) {
      const id = v.getUint16(o, true);
      let m = this.mobs.get(id); if (!m) this.mobs.set(id, m = { id, kind: v.getUint16(o + 2, true), states: [], seen: 0, obj: null, phase: Math.random() * 6.28, breath: Math.random() * 6.28, amp: 0, gy: null, gyT: 0 });
      m.states.push({ t: now, x: v.getFloat32(o + 4, true), y: v.getFloat32(o + 8, true), z: v.getFloat32(o + 12, true), yaw: v.getFloat32(o + 16, true), speed: v.getUint8(o + 20) / 10, flags: v.getUint8(o + 21) });
      if (m.states.length > 4) m.states.shift(); m.seen = now;
    }
  }

  // one template per model file; every mob clones it (shared geometry / materials)
  template(file) {
    if (!this.tpl.has(file)) this.tpl.set(file, this.gltf.loadAsync(`/converted/animated/${file}.glb`).then(g => {
      return prepareTemplate(g.scene);
    }, () => null));
    return this.tpl.get(file);
  }
  async build(m) {
    m.building = true; this.building++;
    try {
      const files = (await this.models)[m.kind]?.models || [], root = new THREE.Group();
      const bones = {}, parts = []; let ref = null, face = null;
      for (const f of files) {
        const t = await this.template(f); if (!t) continue;
        const o = cloneSkinned(t); root.add(o); collectBones(o, t, bones);
        const clips = await loadClips(f), part = clips ? bindClips(o, clips, t.userData.flip) : null;
        if (part) uprightClips(o, part);
        if (!this.face.has(f)) this.face.set(f, facing(o, part));
        face ??= this.face.get(f);
        if (part) { parts.push(part); if (!ref) { if (!this.stride.has(f)) this.stride.set(f, walkSpeed(o, part)); ref = this.stride.get(f); } }
      }
      root.scale.setScalar(AVATAR_SCALE); root.rotation.y = face < 0 ? Math.PI : 0;   // the meshes face either way along z; the server's yaw 0 is travel towards +Z (checked against live mob movement)
      const holder = new THREE.Group(); holder.add(root);
      if (this.mobs.get(m.id) === m) { m.obj = holder; m.model = root; m.face = face ?? -1; m.bones = bones; m.parts = parts.length ? parts : null; m.refSpeed = ref ? ref * AVATAR_SCALE : null; m.rigged = Object.keys(bones).some(k => /^[LR](Leg|Arm)1$/.test(k)); } else m.dead = true;
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
        // stand on whatever walkable surface is within a step of where the mob already is (terrain, floors, stairs, bridges), like the player does; never on roofs
        if (now - m.gyT > 0.1 || m.gy == null) {
          const g = this.world.groundHeight(st.x, st.z);
          if (g != null && g > -1e8) { const f = this.probe.floor(st.x, st.z, (m.gy ?? g) + STEP_UP); m.gy = f > -1e8 ? f : g; }
          m.gyT = now;
        }
        if (m.gy != null) { m.gyS = m.gyS == null ? m.gy : m.gyS + (m.gy - m.gyS) * Math.min(1, dt * 14); st.y = m.gyS; }
      }
      if (!m.shown) { this.scene.add(m.obj); m.shown = true; }
      const dist = Math.sqrt(d2);
      m.obj.position.set(st.x, st.y, st.z); m.obj.rotation.y = st.yaw;
      if (m.height == null) m.height = new THREE.Box3().setFromObject(m.obj).getSize(new THREE.Vector3()).y;
      const moving = st.speed > 0.15; m.amp += ((moving ? 1 : 0) - m.amp) * Math.min(1, dt * 6);
      if (moving) m.phase += dt * Math.PI * st.speed / (0.4 * Math.max(0.8, m.height));   // ~0.4 body heights per step; the phase covers two steps
      m.breath += dt;
      if (m.parts) this.animate(m, st, dt, dist);
      else if (m.rigged) { if (dist < POSE_NEAR || !m.posed) { gait(m, Math.sin(m.phase), Math.cos(m.phase), m.amp); m.posed = true; } }
      else { m.obj.position.y += Math.abs(Math.sin(m.phase)) * 0.05 * m.amp; m.model.rotation.z = Math.sin(m.phase) * 0.04 * m.amp; }   // spiders, serpents, ...: no leg bones to swing, so bob and sway
      const shadow = dist < SHADOW_NEAR; if (shadow !== m.cast) { m.cast = shadow; m.obj.traverse(n => { if (n.isMesh) n.castShadow = shadow; }); }
      if (dist < TAG_NEAR) {
        if (!m.tag) { const t = this.tagFor(m.kind); m.tag = new THREE.Sprite(t.mat); m.tag.renderOrder = 20; m.tag.center.set(0.5, 0); m.tag.userData.aspect = t.aspect; }
        if (!m.tag.parent) this.scene.add(m.tag);
        const th = 20 * fovK * dist; m.tag.position.set(st.x, st.y + m.height + 0.2, st.z); m.tag.scale.set(th * m.tag.userData.aspect, th, 1);
      } else if (m.tag?.parent) this.scene.remove(m.tag);
    }
  }

  /** Idle / walk clips: the one for the current ground speed, crossfaded from the previous one. The walk plays at the rate that keeps the feet on the ground. */
  animate(m, st, dt, dist) {
    const part = m.parts[0], c = part.clips;
    let want = st.speed > 0.15 && c.walk ? 'walk' : 'idle';
    if (!c[want]) want = c.walk ? 'walk' : 'idle';
    m.t ||= {};
    if (m.clip == null) { m.clip = want; m.t[want] = Math.random() * c[want].period; m.fade = 1; }
    if (want !== m.clip) { m.prev = m.clip; m.clip = want; m.fade = 0; m.t[want] ??= 0; }
    const rate = want === 'walk' && m.refSpeed ? Math.min(2.2, Math.max(0.45, st.speed / m.refSpeed)) : 1;
    for (const k of [m.clip, m.prev]) if (k) m.t[k] = (m.t[k] + dt * (k === 'idle' ? 1 : rate)) % c[k].period;
    if (dist >= POSE_NEAR && m.posed) return;
    m.posed = true; m.fade = Math.min(1, m.fade + dt / FADE);
    for (const p of m.parts) {
      if (m.fade < 1 && m.prev && p.clips[m.prev]) { applyClip(p, m.prev, m.t[m.prev] / p.clips[m.prev].dt); applyClip(p, m.clip, m.t[m.clip] / p.clips[m.clip].dt, m.fade); }
      else applyClip(p, m.clip, m.t[m.clip] / p.clips[m.clip].dt);
    }
    if (m.fade >= 1) m.prev = null;
  }

  /** Radar points: x/z of every streamed mob. */
  each(cb) { for (const m of this.mobs.values()) { const s = m.states[m.states.length - 1]; cb(s.x, s.z); } }
}
