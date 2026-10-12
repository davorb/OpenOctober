// Player avatars: the game's racial body-part meshes (head / torso / legs / arms, skinned to one shared skeleton) assembled per race+gender,
// skin-tinted from the head texture, with a small procedural walk cycle.
//   /converted/animated/<race>_<g>_<part>.glb   race: hum alf dwf ork mir mhm   g: m f   (hum_* is the human set; hands are first-person only, so unused)
//   /converted/textures/a*_<race>_<g>_head.png  real face skins. The torso/legs/arms textures in the data are armour atlases / unusable, so
//   the body takes the average colour of the head skin and a per-player cloth colour.
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';

export const RACES = [
  { id: 'hum', name: 'Human' }, { id: 'alf', name: 'Alfar' }, { id: 'dwf', name: 'Dwarf' },
  { id: 'ork', name: 'Orc' }, { id: 'mir', name: 'Mirdain' }, { id: 'mhm', name: 'Mahirim' },
];
const HEAD_TEX = { hum_m: 'a1_hum_m_head', alf_m: 'a9_alf_m_head', alf_f: 'a1_alf_f_head', dwf_m: 'a15_dwf_m_head', dwf_f: 'a15_dwf_f_head',
  ork_m: 'a13_ork_m_head', ork_f: 'a1_ork_f_head', mir_m: 'a10_mir_m_head', mhm_m: 'a10_mhm_m_head', mhm_f: 'a13_mhm_f_head' };
// the face skins are near-neutral; the game tints them per race, so the viewer does too (head map multiplied, body flat)
const TINT = { hum: 0xffffff, alf: 0xd4d6ff, dwf: 0xffe2c8, ork: 0xa4d27c, mir: 0xaed0f0, mhm: 0xf0c48c };
const FILE = { hum: 'hum', alf: 'alfar', dwf: 'dwarf', ork: 'ork', mir: 'mirdain', mhm: 'mahirim' };   // mesh file prefix (textures use the short ids)
const PARTS = ['legs', 'torso', 'arms', 'head'];
const CLOTH = [0x7a2e2e, 0x2e4a7a, 0x3b6b3b, 0x6b5a2e, 0x5a2e6b, 0x2e6b6b, 0x4a4a4a, 0x8a6a3a];

// model units are inches-ish (human ~68 tall); the world is scaled so a 1.75 m human reads as the player's PLAYER_SCALE 1.5 giant
export const AVATAR_SCALE = 0.0254 * 1.5;
export const AVATAR_HEIGHT = 69 * AVATAR_SCALE;

const AX_X = new THREE.Vector3(1, 0, 0), AX_Y = new THREE.Vector3(0, 1, 0), AX_Z = new THREE.Vector3(0, 0, 1);
const qa = new THREE.Quaternion(), qb = new THREE.Quaternion(), qc = new THREE.Quaternion();

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
  constructor() { this.gltf = new GLTFLoader(); this.parts = new Map(); this.skins = new Map(); this.mats = new Map(); this.tl = new THREE.TextureLoader(); }

  // one template per race/gender/part; poses need each bone's rest local rotation and its parent's rest world rotation
  part(race, g, part) {
    const key = `${race}_${g}_${part}`;
    if (!this.parts.has(key)) this.parts.set(key, this.gltf.loadAsync(`/converted/animated/${FILE[race]}_${g}_${part}.glb`).then(gl => {
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
    return this.parts.get(key);
  }

  // face skin + its average colour (for the body); falls back to the other gender's skin of the same race, then to a flat tone
  skin(race, g) {
    const k = `${race}_${g}`; if (this.skins.has(k)) return this.skins.get(k);
    const file = HEAD_TEX[k] || HEAD_TEX[`${race}_${g === 'm' ? 'f' : 'm'}`];
    const p = !file ? Promise.resolve({ map: null, color: new THREE.Color(0xc89a7c) }) : new Promise(res => {
      this.tl.load(`/converted/textures/${file}.png`, t => {
        t.flipY = false; t.colorSpace = THREE.SRGBColorSpace; t.anisotropy = 4;
        const c = document.createElement('canvas'); c.width = c.height = 8; const x = c.getContext('2d'); x.drawImage(t.image, 0, 0, 8, 8);
        const d = x.getImageData(0, 0, 8, 8).data; let r = 0, gg = 0, b = 0, n = 0;
        for (let i = 0; i < d.length; i += 4) { r += d[i]; gg += d[i + 1]; b += d[i + 2]; n++; }
        res({ map: t, color: new THREE.Color().setRGB(r / n / 255, gg / n / 255, b / n / 255, THREE.SRGBColorSpace) });
      }, undefined, () => res({ map: null, color: new THREE.Color(0xc89a7c) }));
    });
    this.skins.set(k, p); return p;
  }

  mat(key, make) { if (!this.mats.has(key)) this.mats.set(key, make()); return this.mats.get(key); }

  /** -> { root, rigs, ... }. `seed` picks the cloth colour. Resolves once every part has loaded. */
  async build(race, g, seed = 0) {
    const [skin, ...tpl] = await Promise.all([this.skin(race, g), ...PARTS.map(p => this.part(race, g, p))]);
    const cloth = CLOTH[Math.abs(seed) % CLOTH.length], k = `${race}_${g}`;
    const skinMat = this.mat('s' + k, () => new THREE.MeshStandardMaterial({ color: skin.color.clone().multiply(new THREE.Color(TINT[race])), roughness: 0.85, side: THREE.DoubleSide }));
    const headMat = this.mat('h' + k, () => new THREE.MeshStandardMaterial({ map: skin.map, color: skin.map ? TINT[race] : skin.color, roughness: 0.8, side: THREE.DoubleSide }));
    const std = (color, r = 0.9) => new THREE.MeshStandardMaterial({ color, roughness: r, side: THREE.DoubleSide });
    const clothMat = this.mat('c' + cloth, () => std(cloth)), pantsMat = this.mat('p' + cloth, () => std(new THREE.Color(cloth).multiplyScalar(0.55))), bootMat = this.mat('boot', () => std(0x3a2c20));
    const root = new THREE.Group(); root.scale.setScalar(AVATAR_SCALE);
    const rigs = [];
    PARTS.forEach((p, i) => {
      const o = cloneSkinned(tpl[i]); o.userData = { rig: tpl[i].userData.rig, bones: {} };
      o.traverse(n => {
        if (n.isBone) { const kk = n.name.replace(/^[a-z]+_f/, ''); if (POSE_BONES.includes(kk)) o.userData.bones[kk] = n; }
        if (!n.isMesh) return;
        n.frustumCulled = false;                                   // bind-pose bounds don't follow the pose; avatars are culled by distance instead
        const sub = /_(\d+)$/.exec(n.material?.name || '')?.[1];   // legs: _0 legs, _1 feet; arms: _0 arms, _1 hands
        n.material = p === 'head' ? headMat : p === 'torso' ? clothMat : p === 'legs' ? (sub === '1' ? bootMat : pantsMat) : skinMat;
      });
      rigs.push(o); root.add(o);
    });
    return { root, rigs, phase: Math.random() * 6.28, meshes: [], race, g };
  }
}

// ---- procedural pose. Every delta is a rotation about a fixed avatar-space axis (model faces -Z, X = sideways, Y = up),
// converted into each bone's local frame through its parent's rest rotation, so the T-pose bone axes don't matter.
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
    put(B.LLeg1, R.LLeg1, qa.setFromAxisAngle(AX_X, air ? 0.25 : legF));
    put(B.RLeg1, R.RLeg1, qa.setFromAxisAngle(AX_X, air ? -0.25 : -legF));
    put(B.LLeg2, R.LLeg2, qa.setFromAxisAngle(AX_X, -kneeL * (air ? 0.4 : 1)));
    put(B.RLeg2, R.RLeg2, qa.setFromAxisAngle(AX_X, -kneeR * (air ? 0.4 : 1)));
    // arms: drop out of the T-pose about Z, then swing about X (applied outermost)
    put(B.LArm1, R.LArm1, qa.setFromAxisAngle(AX_X, -armF).multiply(qb.setFromAxisAngle(AX_Z, -drop)));
    put(B.RArm1, R.RArm1, qa.setFromAxisAngle(AX_X, armF).multiply(qb.setFromAxisAngle(AX_Z, drop)));
    // elbows bend forward: in the T-pose frame that is a turn about Y (opposite sign per side)
    put(B.LArm20, R.LArm20, qa.setFromAxisAngle(AX_Y, -bend));
    put(B.RArm20, R.RArm20, qa.setFromAxisAngle(AX_Y, bend));
  }
}
