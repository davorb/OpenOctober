// Dynamic lights: like the game, only the nearest N light sources are lit per frame (uniform table read by terrain and object shaders).
// Sources come from the placed props (torches, braziers, fireplaces, lamp poles, lanterns) -> data/props.json.
import * as THREE from 'three';

export const MAXL = 16;
//            colour (sRGB)        radius  intensity
const KINDS = [{ c: [1.0, 0.52, 0.18], r: 24, i: 1.9 },   // open fire
               { c: [1.0, 0.58, 0.22], r: 18, i: 1.5 },   // torch
               { c: [1.0, 0.80, 0.50], r: 22, i: 1.2 }];  // lamp / lantern / light pole

export function addLightUniforms(u) {
  u.uDynW = { value: Array.from({ length: MAXL }, () => new THREE.Vector3()) };
  u.uDynV = { value: Array.from({ length: MAXL }, () => new THREE.Vector3()) };
  u.uDynCol = { value: Array.from({ length: MAXL }, () => new THREE.Color(0, 0, 0)) };
  u.uDynRad = { value: new Array(MAXL).fill(0) };
}

export class DynLights {
  constructor(env, world) { this.env = env; this.world = world; this.count = MAXL; this.timer = 0; this.sel = []; this.active = 0; }
  update(dt, camera) {
    const u = this.env.u, T = u.uTime.value;
    this.timer -= dt;
    if (this.timer <= 0) {
      this.timer = 0.15; const cands = [], cx = camera.position.x, cz = camera.position.z;
      for (const t of this.world.tiles.values()) {
        if (!t.lights || t.oState !== 2 || t.dist > 600) continue;
        const L = t.lights;
        for (let i = 0; i < L.length; i += 4) {
          const dx = L[i] - cx, dz = L[i + 2] - cz, d2 = dx * dx + dz * dz;
          if (d2 < 150 * 150) cands.push([d2, i, L]);
        }
      }
      cands.sort((a, b) => a[0] - b[0]);
      this.sel = cands.slice(0, this.count).map(([d2, i, L]) => ({ x: L[i], y: L[i + 1], z: L[i + 2], k: L[i + 3], d2 }));
    }
    const night = 1 - Math.min(1, Math.max(0, ((this.env.state.daylight ?? 1) - 0.1) / 0.6));
    const view = camera.matrixWorldInverse;
    for (let n = 0; n < MAXL; n++) {
      const l = this.sel[n];
      if (!l || n >= this.count) { u.uDynRad.value[n] = 0; continue; }
      const K = KINDS[l.k], ph = l.x * 12.9898 + l.z * 78.233;
      const flick = l.k === 2 ? 1 : 0.86 + 0.14 * Math.sin(T * 9 + ph) * Math.sin(T * 5.3 + ph * 2) + 0.05 * Math.sin(T * 23 + ph);
      const k = K.i * (0.3 + 0.7 * night) * flick;
      u.uDynW.value[n].set(l.x, l.y, l.z);
      u.uDynV.value[n].set(l.x, l.y, l.z).applyMatrix4(view);
      u.uDynCol.value[n].setRGB(K.c[0], K.c[1], K.c[2], THREE.SRGBColorSpace).multiplyScalar(k * (1 - Math.min(1, Math.sqrt(l.d2) / 150) * 0.15));
      u.uDynRad.value[n] = K.r;
    }
  }
}
