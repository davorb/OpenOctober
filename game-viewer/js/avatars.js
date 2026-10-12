// Player avatars, composed the way the game does it (data/paperdoll.json <- common/paperdoll/*.xml, see tools/build_paperdoll.js):
// the naked body of a race+gender is four skinned part meshes sharing one skeleton (converted/animated/<race>_<g>_<part>.glb) with the game's skin
// textures (export/texture/dds, served at /dds/), plus hair as a static mesh on the head frame. A worn armour set swaps part textures / meshes and hangs
// static pieces (helmet, shoulders, ...) on skeleton frames. Not ported: armour dye colour masks, the game's skin/metal shaders (plain PBR here).
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { DDSLoader } from 'three/addons/loaders/DDSLoader.js';

export const RACES = [
  { id: 'hum', name: 'Human' }, { id: 'alf', name: 'Alfar' }, { id: 'dwf', name: 'Dwarf' },
  { id: 'ork', name: 'Orc' }, { id: 'mir', name: 'Mirdain' }, { id: 'mhm', name: 'Mahirim' },
];
// armour sets offered in the picker (the data also has monster / mastery sets that make no sense for a player)
export const GEAR = ['Cloth', 'Leather', 'Padded', 'Studded', 'Chain', 'Scale', 'Banded', 'Plate', 'Full Plate', 'Bone', 'Barbaric', 'Dragon', 'Black Knight', 'Infernal', 'Melechars', 'Severii Centurion'];
const SUBSET = { 'Subset Torso': 'torso', 'Subset Arms': 'arms', 'Subset Legs': 'legs', 'Subset Head': 'head' };
const PARTS = ['legs', 'torso', 'arms', 'head'];

// model units are inches (a human is ~68 tall); the world is scaled so that reads as the player's PLAYER_SCALE 1.5 giant
export const AVATAR_SCALE = 0.0254 * 1.5;
export const AVATAR_HEIGHT = 69 * AVATAR_SCALE;
const ATTACH_SCALE = 1 / 0.0254;                  // attached pieces are modelled in metres, bones live in inches

const AX_X = new THREE.Vector3(1, 0, 0), AX_Y = new THREE.Vector3(0, 1, 0), AX_Z = new THREE.Vector3(0, 0, 1);
const qa = new THREE.Quaternion(), qb = new THREE.Quaternion();
const base = f => f.replace(/\\/g, '/').replace(/^.*\//, '').replace(/\.(sfam|xml|sfm)$/i, '');

function cloneSkinned(src) {
  const clone = src.clone(true), a = [], b = [];
  src.traverse(n => a.push(n)); clone.traverse(n => b.push(n));
  const map = new Map(a.map((n, i) => [n, b[i]]));
  a.forEach((n, i) => {
    if (!n.isSkinnedMesh) return;
    const sk = new THREE.Skeleton(n.skeleton.bones.map(x => map.get(x)), n.skeleton.boneInverses);
    b[i].bind(sk, n.bindMatrix);
  });
  return clone;
}

const POSE_BONES = ['LLeg1', 'RLeg1', 'LLeg2', 'RLeg2', 'LArm1', 'RArm1', 'LArm20', 'RArm20'];

export class AvatarFactory {
  constructor() { this.gltf = new GLTFLoader(); this.dds = new DDSLoader(); this.parts = new Map(); this.statics = new Map(); this.tex = new Map(); this.mats = new Map(); this.pd = null; }

  paperdoll() { return this.pd ||= fetch('/data/paperdoll.json').then(r => r.json()); }
  /** armour sets this race/gender can wear (from the game data), in picker order */
  async gearFor(race, g) { const c = (await this.paperdoll()).chars[`${race}_${g}`]; return GEAR.filter(n => c.sets[n]); }

  // one template per mesh file; poses need each bone's rest local rotation and its parent's rest world rotation
  part(name) {
    if (!this.parts.has(name)) this.parts.set(name, this.gltf.loadAsync(`/converted/animated/${name}.glb`).then(gl => {
      const scene = gl.scene; scene.updateMatrixWorld(true);
      scene.userData.rig = {};
      scene.traverse(n => {
        if (!n.isBone) return;
        const k = n.name.replace(/^[a-z]+_f/, '');
        if (!POSE_BONES.includes(k)) return;
        const pq = new THREE.Quaternion(); n.parent.getWorldQuaternion(pq);
        scene.userData.rig[k] = { q0: n.quaternion.clone(), pq, pqInv: pq.clone().invert() };
      });
      return scene;
    }));
    return this.parts.get(name);
  }
  // attached pieces are static meshes: models/ first, animated/ for the ones converted with the character set
  static(name) {
    if (!this.statics.has(name)) this.statics.set(name, this.gltf.loadAsync(`/converted/models/${name}.glb`).catch(() => this.gltf.loadAsync(`/converted/animated/${name}.glb`)).then(g => g.scene, () => null));
    return this.statics.get(name);
  }
  texture(name) {
    if (!this.tex.has(name)) this.tex.set(name, new Promise(res => this.dds.load(`/dds/${name}.dds`, t => {
      t.colorSpace = THREE.SRGBColorSpace; t.wrapS = t.wrapT = THREE.RepeatWrapping; t.anisotropy = 4; t.needsUpdate = true; res(t);
    }, undefined, () => res(null))));
    return this.tex.get(name);
  }
  mat(name, kind) {
    const key = name + '|' + kind;
    if (!this.mats.has(key)) this.mats.set(key, this.texture(name).then(map => {
      const worn = !name.startsWith('a1_'), m = new THREE.MeshStandardMaterial({ map, color: map ? 0xffffff : 0x8a8478, roughness: worn ? 0.55 : 0.8, metalness: worn ? 0.25 : 0, side: THREE.DoubleSide });
      if (kind === 'hair') { m.alphaTest = 0.45; m.roughness = 0.9; }
      return m;
    }));
    return this.mats.get(key);
  }

  /** What a character shows: { parts: { torso: { file, mats: {id: texture} }, ... }, attach: { point: item } } for a race/gender wearing `gear` ('' = naked). */
  async compose(race, g, gear) {
    const c = (await this.paperdoll()).chars[`${race}_${g}`], parts = {}, attach = {};
    const apply = list => { for (const it of list) {
      if (it.a) { attach[it.a] = it; continue; }
      const k = SUBSET[it.s]; if (!k) continue;
      const p = parts[k] ||= { file: null, mats: {} };
      if (it.f !== 'NULL') p.file = base(it.f);
      for (const [id, t] of it.m) p.mats[id] = t;
    } };
    apply(c.body);
    const body = JSON.parse(JSON.stringify(parts));
    for (const n of (gear && c.sets[gear]) || []) apply(c.items[n]);
    return { parts, body, attach, c };
  }

  /** -> { root, rigs, phase, ... }. Resolves once every piece has loaded. */
  async build(race, g, gear = '') {
    const { parts, body, attach } = await this.compose(race, g, gear), A = (await this.paperdoll()).attach;
    const root = new THREE.Group(); root.scale.setScalar(AVATAR_SCALE);
    const rigs = [];
    for (const p of PARTS) {
      const spec = parts[p]; if (!spec?.file) continue;
      let tpl = await this.part(spec.file).catch(() => null), mats = spec.mats;
      if (!tpl && body[p]?.file) { tpl = await this.part(body[p].file); mats = body[p].mats; }   // armour mesh missing from the export: wear the body mesh
      if (!tpl) continue;
      const o = cloneSkinned(tpl); o.userData = { rig: tpl.userData.rig, bones: {} };
      const jobs = [];
      o.traverse(n => {
        if (n.isBone) { const kk = n.name.replace(/^[a-z]+_f/, ''); if (POSE_BONES.includes(kk)) o.userData.bones[kk] = n; }
        if (!n.isMesh) return;
        n.frustumCulled = false;                                   // bind-pose bounds don't follow the pose; avatars are culled by distance instead
        const id = +(/_(\d+)$/.exec(n.material?.name || '')?.[1] ?? 0), tex = mats[id] ?? mats[0];
        jobs.push(this.mat(tex, 'body').then(m => { n.material = m; }));
      });
      await Promise.all(jobs); rigs.push(o); root.add(o);
    }
    // attached pieces (hair, helmet, shoulders, ...): static meshes parented to a skeleton frame
    const host = rigs.find(r => r.userData.bones.LLeg1) || rigs[0];
    for (const [point, it] of Object.entries(attach)) {
      const ap = A[point]; if (!ap || !host) continue;
      const bone = host.getObjectByName(ap.frame); if (!bone) continue;
      const src = await this.static(base(it.f)); if (!src) continue;
      const mesh = src.clone(true), holder = new THREE.Group();
      holder.position.fromArray(ap.t); holder.rotation.set(ap.r[0], ap.r[1], ap.r[2]); holder.scale.setScalar(ATTACH_SCALE); holder.add(mesh);
      const hair = point === 'AttachPoint_Hair', jobs = [];
      mesh.traverse(n => { if (n.isMesh) { n.frustumCulled = false; const id = +(/_(\d+)$/.exec(n.material?.name || '')?.[1] ?? 0), tex = (it.m.find(m => m[0] === id) || it.m[0])?.[1]; if (tex) jobs.push(this.mat(tex, hair ? 'hair' : 'worn').then(m => { n.material = m; })); } });
      await Promise.all(jobs); bone.add(holder);
    }
    return { root, rigs, phase: Math.random() * 6.28, race, g, gear };
  }
}

// ---- procedural pose. Every delta is a rotation about a fixed avatar-space axis (model faces -Z, X = sideways, Y = up),
// converted into each bone's local frame through its parent's rest rotation, so the T-pose bone axes don't matter.
// NB the body meshes face +Z (chest/nose extend towards +Z), so "forward" below is +Z; multiplayer.js turns avatars by PI to match the camera's -Z convention.
function put(b, rig, R) { b.quaternion.copy(rig.pqInv).multiply(R).multiply(rig.pq).multiply(rig.q0); }

/** swing: -1..1 stride phase output (sin), amp: 0 idle .. 1 full run, air: arms/legs out */
export function pose(av, swing, amp, air) {
  const sw = swing * amp;
  const legF = 0.75 * sw, armF = 0.6 * sw;
  const kneeL = Math.max(0, -Math.sin(av.phase)) * 1.0 * amp, kneeR = Math.max(0, Math.sin(av.phase)) * 1.0 * amp;
  const drop = air ? 0.9 : 1.3, bend = 0.25 + 0.5 * amp;
  for (const o of av.rigs) {
    const B = o.userData.bones, R = o.userData.rig;
    if (!B.LLeg1) continue;
    put(B.LLeg1, R.LLeg1, qa.setFromAxisAngle(AX_X, air ? -0.25 : -legF));
    put(B.RLeg1, R.RLeg1, qa.setFromAxisAngle(AX_X, air ? 0.25 : legF));
    put(B.LLeg2, R.LLeg2, qa.setFromAxisAngle(AX_X, kneeL * (air ? 0.4 : 1)));
    put(B.RLeg2, R.RLeg2, qa.setFromAxisAngle(AX_X, kneeR * (air ? 0.4 : 1)));
    // arms: drop out of the T-pose about Z, then swing about X (applied outermost)
    put(B.LArm1, R.LArm1, qa.setFromAxisAngle(AX_X, armF).multiply(qb.setFromAxisAngle(AX_Z, -drop)));
    put(B.RArm1, R.RArm1, qa.setFromAxisAngle(AX_X, -armF).multiply(qb.setFromAxisAngle(AX_Z, drop)));
    // elbows bend forward: in the T-pose frame that is a turn about Y (opposite sign per side)
    put(B.LArm20, R.LArm20, qa.setFromAxisAngle(AX_Y, bend));
    put(B.RArm20, R.RArm20, qa.setFromAxisAngle(AX_Y, -bend));
  }
}
