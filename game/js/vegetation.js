// Grass / flower / mushroom tufts around the camera.
// Meshes and the per-tile type tables come from export/veg (tools/build_veg.js -> data/veg.json). The game's own placement (the run-length cell lists in
// terrain_<tile>.sfv and the native scatter code) is NOT decoded, so this is an approximation: tufts are scattered deterministically per 8 m chunk where
// the terrain splat weights are grassy, picking only mesh types that tile's table lists. Shading/wind follow shaders/vegetation.teq (up-facing lighting,
// wind sway weighted by height). Tufts stay full height out to about 62% of the draw distance, then shrink away at the edge.
// Draw distance comes from the performance panel (O).
import * as THREE from 'three';
import { fogUniforms, fogFn } from './chunks.js';
import { SEA_LEVEL } from './water.js';

const CH = 8, DENSITY = 0.55, CAP = 12000;
const GRASS_Y = 1.75 * 2 / 3;   // grass_* tufts, Y only: the 1.75 stretch, then one third shorter. Flowers and mushrooms stay authored.
const CLASS_RE = [/^grass_(green|jungle|moss|central|straws_green)/, /^grass_(yellow|straws_yellow)/, /^grass_swamp/, /^grass_(dead|branches)/, /^(grass_flowers|flower_)/, /^mushroom/];
const classOfMesh = n => CLASS_RE.findIndex(r => r.test(n));
const GW = 257;

const vs = `
attribute vec3 aOff; attribute vec3 aRS;      // xyz offset; (yaw, scale, tint)
uniform float uHeight; uniform vec3 uCam; uniform float uTime; uniform vec3 uWindV; uniform float uF0; uniform float uF1; uniform float uYScale;
varying vec2 vUv; varying vec3 vView; varying float vTint;
void main() {
  float d = distance(aOff.xz, uCam.xz);
  float fade = 1.0 - smoothstep(uF0, uF1, d);
  vec3 p = position * aRS.y; p.y *= uYScale * mix(0.25, 1.0, fade) * step(0.001, fade);
  float hw = clamp(position.y / uHeight, 0.0, 1.0); hw *= hw;
  float c = cos(aRS.x), s = sin(aRS.x);
  p.xz = vec2(c * p.x + s * p.z, -s * p.x + c * p.z);
  float ph = dot(aOff.xz, vec2(0.37, 0.29));
  float sw = sin(uTime * 1.6 + ph) * 0.6 + sin(uTime * 3.3 + ph * 2.1) * 0.4;
  vec2 wd = uWindV.xz; float wl = length(wd); wd = wl > 1e-3 ? wd / wl : vec2(1.0, 0.0);
  float amp = (0.10 + min(wl, 8.0) * 0.03) * aRS.y;
  p.xz += wd * (sw * 0.6 + 0.4) * amp * hw * uHeight * uYScale;
  vec4 mv = viewMatrix * vec4(aOff + p, 1.0);
  vView = mv.xyz; vUv = uv; vTint = aRS.z;
  gl_Position = projectionMatrix * mv;
}`;
const fs = `
${fogUniforms}${fogFn}
uniform sampler2D tMap; uniform vec3 uSunDir; uniform vec3 uAmbSky; uniform vec3 uMoonDir[3]; uniform vec3 uMoonColor[3];
varying vec2 vUv; varying vec3 vView; varying float vTint;
void main() {
  vec4 t = texture2D(tMap, vUv);
  if (t.a < 0.5) discard;
  float sunH = smoothstep(-0.05, 0.12, uSunDir.y + 0.04);
  vec3 lit = uSunColor * max(uSunDir.y, 0.0) * sunH * 0.7;
  for (int m = 0; m < 3; m++) lit += uMoonColor[m] * max(uMoonDir[m].y, 0.0) * 0.7;
  vec3 col = t.rgb * mix(0.85, 1.12, vTint) * (lit + uAmbSky) * 1.6;
  col = dfFog(col, vView, 1.0);
  gl_FragColor = vec4(col, 1.0);
  #include <colorspace_fragment>
}`;

function rng(seed) { let a = seed >>> 0; return () => { a = (a + 0x6D2B79F5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }

export class Vegetation {
  constructor({ scene, env, world, data, distance }) {
    this.env = env; this.world = world; this.data = data; this.enabled = true; this.radius = 0;
    this.group = new THREE.Group(); scene.add(this.group);
    this.chunks = new Map(); this.cx = NaN; this.cz = NaN; this.timer = 0; this.pending = false;
    this.types = new Map();                       // mesh name -> { mesh, attrs, count }
    const loader = new THREE.TextureLoader(), texs = new Map();
    this.tileCache = new Map();                   // tile -> per-class [{ name, w }]
    for (const [name, m] of Object.entries(data.meshes)) {
      const g = new THREE.InstancedBufferGeometry();
      g.setAttribute('position', new THREE.Float32BufferAttribute(m.pos, 3));
      g.setAttribute('uv', new THREE.Float32BufferAttribute(m.uv, 2));
      g.setIndex(m.idx);
      const off = new THREE.InstancedBufferAttribute(new Float32Array(CAP * 3), 3), rs = new THREE.InstancedBufferAttribute(new Float32Array(CAP * 3), 3);
      off.setUsage(THREE.DynamicDrawUsage); rs.setUsage(THREE.DynamicDrawUsage);
      g.setAttribute('aOff', off); g.setAttribute('aRS', rs); g.instanceCount = 0;
      let tex = texs.get(m.tex);
      if (!tex) { tex = loader.load('/converted/textures/' + m.tex + '.png'); tex.colorSpace = THREE.SRGBColorSpace; tex.wrapS = tex.wrapT = THREE.RepeatWrapping; tex.anisotropy = 4; texs.set(m.tex, tex); }
      let hmax = 0.1; for (let i = 1; i < m.pos.length; i += 3) hmax = Math.max(hmax, m.pos[i]);
      const mat = new THREE.ShaderMaterial({ vertexShader: vs, fragmentShader: fs, side: THREE.DoubleSide,
        uniforms: { ...env.u, tMap: { value: tex }, uHeight: { value: hmax }, uYScale: { value: name.startsWith('grass_') ? GRASS_Y : 1 }, uCam: { value: new THREE.Vector3() }, uWindV: env.u.uWind, uF0: { value: 22 }, uF1: { value: 38 } } });
      const mesh = new THREE.Mesh(g, mat); mesh.frustumCulled = false; mesh.visible = false; mesh.renderOrder = 0;
      this.group.add(mesh);
      this.types.set(name, { mesh, off, rs, count: 0, cls: classOfMesh(name) });
    }
    this.camV = new THREE.Vector3();
    this.setDistance(distance);
  }

  // metres. Full height until ~62% of the range, then the same shrink-out the shader already does.
  setDistance(meters) {
    const d = Math.min(160, Math.max(40, +meters || 80));
    if (d === this.radius) return;
    this.radius = d;
    const f0 = d * 0.62, f1 = d - 2;
    for (const t of this.types.values()) { const u = t.mesh.material.uniforms; u.uF0.value = f0; u.uF1.value = f1; }
    this.cx = NaN;
  }

  // per-tile, per-class list of [mesh, cumulative weight] from the tile's type table
  _table(tile) {
    let r = this.tileCache.get(tile); if (r) return r;
    r = Array.from({ length: 6 }, () => []);
    for (const [name, f0] of this.data.tiles[tile] || []) {
      const c = classOfMesh(name); if (c < 0) continue;
      r[c].push([name, Math.max(f0, 0.002)]);
    }
    for (const l of r) { let s = 0; for (const e of l) s += e[1]; let a = 0; for (const e of l) { a += e[1] / s; e[1] = a; } }
    this.tileCache.set(tile, r); return r;
  }

  _chunk(ix, iz) {
    const w = this.world, key = ix + ',' + iz;
    let c = this.chunks.get(key); if (c) return c;
    const x0 = ix * CH, z0 = iz * CH, tile = w.tileAt(x0 + CH / 2, z0 + CH / 2);
    if (!tile || !tile.heights || !tile.vegw) { this.pending = this.pending || (tile && tile.tState === 1); return null; }
    const tab = this._table(tile.name), out = [], R = rng((ix * 73856093) ^ (iz * 19349663) ^ 0x51ED), n = Math.round(DENSITY * CH * CH);
    for (let i = 0; i < n; i++) {
      const x = x0 + R() * CH, z = z0 + R() * CH, pick = R(), yaw = R() * 6.2832, sc = 0.7 + R() * 0.6, tint = R();
      const t = w.tileAt(x, z); if (t !== tile) continue;
      const gx = Math.round((x - (t.rect.x0 + 512)) / 4 + 128), gz = Math.round((z - (t.rect.z0 + 512)) / 4 + 128);
      if (gx < 0 || gz < 0 || gx >= GW || gz >= GW) continue;
      const o = (gz * GW + gx) * 6, v = t.vegw;
      let tot = 0; for (let q = 0; q < 6; q++) tot += v[o + q];
      if (tot < 40 || R() * 255 > tot) continue;                                  // thin out where the grass materials fade
      let u = R() * tot, cls = 0; for (; cls < 5; cls++) { if (u < v[o + cls]) break; u -= v[o + cls]; }
      if (cls === 5 && R() > 0.06) cls = 0;                                         // mushrooms are rare
      let list = tab[cls]; if (!list.length) continue;
      let name = list[list.length - 1][0]; for (const e of list) if (pick <= e[1]) { name = e[0]; break; }
      const y = w.groundHeight(x, z); if (y == null || y < SEA_LEVEL + 0.6) continue;
      const sx = (w.groundHeight(x + 1.5, z) ?? y) - (w.groundHeight(x - 1.5, z) ?? y), sz = (w.groundHeight(x, z + 1.5) ?? y) - (w.groundHeight(x, z - 1.5) ?? y);
      if (Math.hypot(sx, sz) > 2.1) continue;                                       // > ~46 deg
      out.push({ name, x, y: y - 0.05, z, yaw, sc, tint });
    }
    this.chunks.set(key, c = out); return c;
  }

  update(dt, cam) {
    if (!this.enabled) { this.group.visible = false; return; }
    this.group.visible = true;
    const p = cam.position; this.camV.copy(p);
    for (const t of this.types.values()) t.mesh.material.uniforms.uCam.value.copy(p);
    this.timer -= dt;
    const cx = Math.floor(p.x / CH), cz = Math.floor(p.z / CH), moved = cx !== this.cx || cz !== this.cz;
    if (!moved && !(this.pending && this.timer <= 0)) return;
    if (this.timer > 0 && !moved) return;
    this.timer = 0.6; this.cx = cx; this.cz = cz; this.pending = false;
    const gh = this.world.groundHeight(p.x, p.z);
    for (const t of this.types.values()) t.count = 0;
    if (gh != null && p.y - gh < Math.max(100, this.radius)) {
      const r = Math.ceil(this.radius / CH), cells = [];
      for (let dz = -r; dz <= r; dz++) for (let dx = -r; dx <= r; dx++) {
        const dist = Math.hypot((cx + dx + 0.5) * CH - p.x, (cz + dz + 0.5) * CH - p.z);
        if (dist <= this.radius + CH) cells.push([dx, dz, dist]);
      }
      cells.sort((a, b) => a[2] - b[2]);   // near tufts fill the per-mesh cap first
      for (const [dx, dz] of cells) {
        const ch = this._chunk(cx + dx, cz + dz); if (!ch) continue;
        for (const i of ch) {
          const t = this.types.get(i.name); if (!t || t.count >= CAP) continue;
          const k = t.count++;
          t.off.array.set([i.x, i.y, i.z], k * 3); t.rs.array.set([i.yaw, i.sc, i.tint], k * 3);
        }
      }
    }
    for (const t of this.types.values()) { t.mesh.visible = t.count > 0; t.mesh.geometry.instanceCount = t.count; t.off.needsUpdate = t.rs.needsUpdate = true; }
    const keep = Math.ceil(this.radius / CH) + 4;
    if (this.chunks.size > (keep * 2) ** 2) for (const k of this.chunks.keys()) { const [a, b] = k.split(',').map(Number); if (Math.abs(a - cx) > keep || Math.abs(b - cz) > keep) this.chunks.delete(k); }
  }
}
