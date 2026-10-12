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

const _tmp = new THREE.Vector3();
export class DynLights {
  get _d() { return _tmp; }
  constructor(env, world, scene) {
    this.env = env; this.world = world; this.count = MAXL; this.timer = 0; this.sel = []; this.active = 0;
    // roa_lightglow: additive round corona per light (g = (1-d)^2, warm rim -> white core, GLOW_INTENSITY 0.5, GLOW_WHITE 0.35), depth-tested here instead of occlusion queries
    const mat = new THREE.ShaderMaterial({
      uniforms: {}, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, fog: false,
      vertexShader: 'attribute vec3 color3; varying vec3 vC; varying vec2 vT; void main() { vT = uv; vC = color3; gl_Position = projectionMatrix * (modelViewMatrix * vec4(0.0, 0.0, 0.0, 1.0) + vec4(position.xy * length(vec3(modelMatrix[0])), 0.0, 0.0)); }',
      fragmentShader: 'varying vec3 vC; varying vec2 vT; void main() { float g = clamp(1.0 - length(vT - 0.5) * 2.0, 0.0, 1.0); g *= g; gl_FragColor = vec4(mix(vC, vec3(1.0), clamp(g * 0.35, 0.0, 1.0)) * g * 0.5, 1.0);\n#include <colorspace_fragment>\n }',
    });
    this.glows = [];
    if (scene) for (let i = 0; i < MAXL; i++) {
      const g = new THREE.PlaneGeometry(1, 1); g.setAttribute('color3', new THREE.BufferAttribute(new Float32Array(12), 3));
      const m = new THREE.Mesh(g, mat); m.frustumCulled = false; m.renderOrder = 5; m.visible = false; scene.add(m); this.glows.push(m);
    }
  }
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
      const gl = this.glows[n]; if (gl) gl.visible = false;
      if (!l || n >= this.count) { u.uDynRad.value[n] = 0; continue; }
      const K = KINDS[l.k], ph = l.x * 12.9898 + l.z * 78.233;
      const flick = l.k === 2 ? 1 : 0.86 + 0.14 * Math.sin(T * 9 + ph) * Math.sin(T * 5.3 + ph * 2) + 0.05 * Math.sin(T * 23 + ph);
      const k = K.i * (0.3 + 0.7 * night) * flick;
      u.uDynW.value[n].set(l.x, l.y, l.z);
      u.uDynV.value[n].set(l.x, l.y, l.z).applyMatrix4(view);
      u.uDynCol.value[n].setRGB(K.c[0], K.c[1], K.c[2], THREE.SRGBColorSpace).multiplyScalar(k * (1 - Math.min(1, Math.sqrt(l.d2) / 150) * 0.15));
      u.uDynRad.value[n] = K.r;
      if (gl) {   // corona sits on the flame, slightly toward the camera so the torch model does not clip it; brighter at night, size grows a little with distance
        const dist = Math.sqrt(l.d2) + 1, c = u.uDynCol.value[n], a = Math.min(1, 0.35 + night * 0.65) * flick;
        gl.position.set(l.x, l.y + 0.3, l.z).addScaledVector(this._d.set(camera.position.x - l.x, camera.position.y - l.y, camera.position.z - l.z).normalize(), 0.6);
        gl.scale.setScalar((K.r * 0.15 + dist * 0.008) * (0.7 + 0.3 * a)); gl.visible = true;
        const col = gl.geometry.attributes.color3, m = Math.max(c.r, c.g, c.b, 1e-3);
        for (let v = 0; v < 4; v++) col.setXYZ(v, c.r / m * a, c.g / m * a, c.b / m * a); col.needsUpdate = true;
      }
    }
  }
}
