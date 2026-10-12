// Terrain: per-tile geometry from the worker + the splat/triplanar material ported from the game's roa_1d_terrain.teq.
import * as THREE from 'three';
import { DDSLoader } from 'three/addons/loaders/DDSLoader.js';
import { fogUniforms, fogFn } from './chunks.js';

export class TerrainTextures {
  constructor(renderer, manifest) {
    this.manifest = manifest; this.cache = new Map();
    this.s3tc = renderer.extensions.has('WEBGL_compressed_texture_s3tc');
    this.aniso = Math.min(8, renderer.capabilities.getMaxAnisotropy());
    this.dds = new DDSLoader(); this.png = new THREE.TextureLoader();
    // neutral Far-normal payload: flat normal, height 0.5, AO 1
    this.flat = new THREE.DataTexture(new Uint8Array([128, 128, 255, 128]), 1, 1); this.flat.needsUpdate = true;
    this.detail = this._load('terrain_detail_shared.dds', false);
  }
  _load(file, srgb) {
    const key = file + srgb;
    if (this.cache.has(key)) return this.cache.get(key);
    let t;
    if (this.s3tc) t = this.dds.load('/dds/' + file, undefined);
    else { t = this.png.load('/converted/textures/' + file.replace(/\.dds$/, '.png')); }
    t.wrapS = t.wrapT = THREE.RepeatWrapping; t.anisotropy = this.aniso;
    if (srgb) t.colorSpace = THREE.SRGBColorSpace;
    if (this.s3tc) t.minFilter = THREE.LinearMipmapLinearFilter;
    this.cache.set(key, t); return t;
  }
  diffuse(name) { return this._load(name, true); }
  normal(name) { const n = this.manifest[name]; return n && this.s3tc ? this._load(n, false) : null; }
}

const MAXN = 7;
const vs = `
attribute vec4 aW0; attribute vec4 aW1; attribute vec4 aN;
varying vec3 vWorld; varying vec3 vN; varying vec4 vW0; varying vec4 vW1; varying vec3 vView;
void main() {
  vec4 wp = modelMatrix * vec4(position, 1.0);
  vWorld = wp.xyz; vN = normalize(mat3(modelMatrix) * aN.xyz);
  vW0 = aW0; vW1 = aW1;
  vec4 mv = viewMatrix * wp; vView = mv.xyz;
  gl_Position = projectionMatrix * mv;
}`;

// Material-count variants, like the game's terrain_new_<N>mat shaders.
const cache = new Map();
function fragment(N) {
  if (cache.has(N)) return cache.get(N);
  let s = `
#include <packing>
${fogUniforms}${fogFn}
uniform sampler2D tDetail; uniform float uFade; uniform float uTile; uniform float uHB_sharp; uniform float uHB_bias;
uniform vec3 uSunDir; uniform vec3 uAmbSky; uniform vec3 uAmbGround; uniform vec3 uMoonDir[3]; uniform vec3 uMoonColor[3];
uniform float uSlot[${N}]; uniform float uParallax;
uniform sampler2D uShadowMap; uniform mat4 uShadowMat; uniform float uShadowOn; uniform float uShadowTexel;
uniform vec3 uDynW[16]; uniform vec3 uDynCol[16]; uniform float uDynRad[16];
${Array.from({ length: N }, (_, i) => `uniform sampler2D tD${i}; uniform sampler2D tN${i}; uniform float uHasN${i};`).join('\n')}
varying vec3 vWorld; varying vec3 vN; varying vec4 vW0; varying vec4 vW1; varying vec3 vView;
float shadowAt(vec3 wp, vec3 n) {
  if (uShadowOn < 0.5) return 1.0;
  vec3 q = (uShadowMat * vec4(wp + n * 0.7, 1.0)).xyz;
  if (q.x <= 0.0 || q.x >= 1.0 || q.y <= 0.0 || q.y >= 1.0 || q.z >= 1.0) return 1.0;
  float s = 0.0;
  for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) s += step(q.z - 0.0005, unpackRGBAToDepth(texture2D(uShadowMap, q.xy + vec2(float(x), float(y)) * uShadowTexel)));
  float edge = smoothstep(0.0, 0.1, min(min(q.x, 1.0 - q.x), min(q.y, 1.0 - q.y)));
  return mix(1.0, s / 9.0, edge);
}
float slotW(float s) { int i = int(s + 0.5); return i < 4 ? vW0[i] : vW1[i - 4]; }
void main() {
  vec3 n = normalize(vN);
  float dist = length(vView);
  vec3 p = vWorld * uTile;                       // 0.058 -> a material repeat every ~17 m, world anchored so tiles join seamlessly
  vec3 dx = dFdx(p), dy = dFdy(p);
  // triplanar weights (steep faces use the vertical projections)
  vec3 bw = pow(max(abs(n) - vec3(0.2, 0.7, 0.2), vec3(0.0)), vec3(3.0));
  bw /= max(dot(bw, vec3(1.0)), 1e-5);
  float w[${N}]; float h[${N}]; float ao[${N}]; vec2 nm[${N}]; vec3 alb[${N}]; vec3 albG[${N}];
  float wTotal = 0.0;
`;
  for (let i = 0; i < N; i++) s += `
  w[${i}] = slotW(uSlot[${i}]); wTotal += w[${i}];
  albG[${i}] = textureGrad(tD${i}, p.xz, dx.xz, dy.xz).rgb;
  alb[${i}] = albG[${i}] * bw.y;
  if (bw.x > 0.003) alb[${i}] += textureGrad(tD${i}, p.zy, dx.zy, dy.zy).rgb * bw.x;
  if (bw.z > 0.003) alb[${i}] += textureGrad(tD${i}, p.xy, dx.xy, dy.xy).rgb * bw.z;
  h[${i}] = 0.5; ao[${i}] = 1.0; nm[${i}] = vec2(0.0);
  if (uHasN${i} > 0.5) { vec4 f = textureGrad(tN${i}, p.xz, dx.xz, dy.xz); h[${i}] = f.r; ao[${i}] = f.b; nm[${i}] = f.ag * 2.0 - 1.0; }
`;
  s += `
  // height-aware material contest: taller material wins the band between two painted weights
  float hwSum = 0.0; float hw[${N}];
`;
  for (let i = 0; i < N; i++) s += `  hw[${i}] = w[${i}] * pow(h[${i}] + uHB_bias, uHB_sharp); hwSum += hw[${i}];\n`;
  s += `
  vec3 col = vec3(0.0); float aoB = 0.0; vec2 nB = vec2(0.0);
  float inv = hwSum > 1e-6 ? 1.0 / hwSum : 0.0;
`;
  for (let i = 0; i < N; i++) s += `  { float ws = hw[${i}] * inv; col += alb[${i}] * ws; aoB += ao[${i}] * ws; nB += nm[${i}] * ws; }\n`;
  s += `
  if (hwSum <= 1e-6) { col = alb[0]; aoB = 1.0; }
  // Parallax, after the game's PHASE 3a block in roa_1d_terrain.teq (shipped disabled): single-step, offset-limited, ground (XZ) leg only,
  //   pOfs = toEye.xz/|toEye| * (height - 0.5) * 0.020 * smoothstep(12, 3, dist)    (material-UV units; no divide by toEye.y)
  // Blend weights stay the unoffset ones; only the surface is resampled. The game's baked height payload (normal map .r) is a constant 255 in
  // the shipped files, so the height here is a proxy: the unoffset albedo's perceptual brightness.
  float pFade = smoothstep(12.0, 3.0, dist) * uParallax;
  if (pFade > 0.0 && bw.y > 0.003 && hwSum > 1e-6) {
    float ph = sqrt(clamp(dot(col, vec3(0.299, 0.587, 0.114)), 0.0, 1.0));
    vec2 pOfs = (cameraPosition.xz - vWorld.xz) / max(dist, 1e-3) * ((ph - 0.5) * 0.020 * pFade);
    vec3 dCol = vec3(0.0); vec2 dN = vec2(0.0);
`;
  for (let i = 0; i < N; i++) s += `    { float ws = hw[${i}] * inv * bw.y; dCol += (textureGrad(tD${i}, p.xz + pOfs, dx.xz, dy.xz).rgb - albG[${i}]) * ws;
      if (uHasN${i} > 0.5) dN += (textureGrad(tN${i}, p.xz + pOfs, dx.xz, dy.xz).ag * 2.0 - 1.0 - nm[${i}]) * ws; }\n`;
  s += `    col += dCol; nB += dN;
  }
  // shared detail texture: .r macro brightness (fades in with distance, breaks tiling), .ag micro normal (near field only)
  vec2 duv = vWorld.xz * 0.7;
  vec4 mac = texture2D(tDetail, vWorld.xz / 650.0);
  duv += (mac.b - 0.5) * 0.35;
  vec4 d1 = texture2D(tDetail, duv); vec4 d2 = texture2D(tDetail, duv * 0.37 + 0.31);
  vec2 dn = mix(d1.ag, d2.ag, 0.4) * 2.0 - 1.0;
  float nearF = 1.0 - smoothstep(25.0, 140.0, dist);
  float farF = smoothstep(60.0, 450.0, dist);
  col *= mix(1.0, 0.85 + 0.30 * mac.r, farF);
  // perturb the normal with the baked Far normal map (ground leg) + micro detail
  vec3 nn = normalize(n + vec3(nB.x, 0.0, nB.y) * bw.y * 0.9 + vec3(dn.x, 0.0, dn.y) * 0.4 * nearF * bw.y);
  // lighting: sun + moons + hemispheric ambient scaled by the baked AO (0.65 strength, eased at twilight)
  float sunH = clamp(uSunDir.y, 0.0, 1.0);
  float aoStrength = 0.65 * mix(0.35, 1.0, smoothstep(0.0, 0.2588, sunH));
  float aoT = mix(1.0, aoB, aoStrength);
  vec3 amb = mix(uAmbGround, uAmbSky, nn.y * 0.5 + 0.5) * aoT;
  vec3 lit = uSunColor * max(dot(nn, uSunDir), 0.0) * smoothstep(-0.05, 0.12, uSunDir.y + 0.04) * shadowAt(vWorld, n);
  for (int m = 0; m < 3; m++) lit += uMoonColor[m] * max(dot(nn, uMoonDir[m]), 0.0);
  for (int i = 0; i < 16; i++) { if (uDynRad[i] <= 0.0) continue; vec3 d = uDynW[i] - vWorld; float dd = length(d); float att = clamp(1.0 - dd / uDynRad[i], 0.0, 1.0); lit += uDynCol[i] * att * att * (max(dot(nn, d / dd), 0.0) * 0.8 + 0.2); }
  col *= (lit + amb) * 1.6;
  col = dfFog(col, vView, uFade);
  gl_FragColor = vec4(col, 1.0);
  #include <colorspace_fragment>
}`;
  cache.set(N, s); return s;
}

const blankTex = new THREE.DataTexture(new Uint8Array([128, 128, 128, 255]), 1, 1); blankTex.needsUpdate = true;

export function makeTerrainMaterial(group, tex, env, fade, fallback) {
  let pairs = group.textures.map((t, i) => [t, group.slots[i]]).filter(([t, s]) => t && s != null).slice(0, MAXN);
  // groups whose texture refs are all unresolved would render as the flat blank texture (bright grey under the 1.6x lighting): borrow the tile's main texture
  if (!pairs.length && fallback) pairs = [[fallback, group.slots[0] ?? 0]];
  const N = Math.max(1, pairs.length);
  const uniforms = {
    ...env.u, tDetail: { value: tex.detail }, uFade: fade, uTile: { value: 0.058 },
    uHB_sharp: { value: 3.0 }, uHB_bias: { value: 0.30 }, uParallax: env.u.uParallax,
    uSlot: { value: Array.from({ length: N }, (_, i) => pairs[i]?.[1] ?? 0) },
  };
  for (let i = 0; i < N; i++) {
    const name = pairs[i]?.[0];
    uniforms['tD' + i] = { value: name ? tex.diffuse(name) : blankTex };
    const nt = name ? tex.normal(name) : null;
    uniforms['tN' + i] = { value: nt || tex.flat };
    uniforms['uHasN' + i] = { value: nt ? 1 : 0 };
  }
  return new THREE.ShaderMaterial({ vertexShader: vs, fragmentShader: fragment(N), uniforms, side: THREE.FrontSide, extensions: { derivatives: true } });
}

// Build the THREE objects for one parsed tile
export function buildTerrain(data, tex, env, fade) {
  const g = new THREE.Group();
  const pos = new THREE.BufferAttribute(data.pos, 3);
  const aN = new THREE.BufferAttribute(data.nrm, 4, true);
  const aW0 = new THREE.BufferAttribute(data.w0, 4, true), aW1 = new THREE.BufferAttribute(data.w1, 4, true);
  const box = new THREE.Box3(new THREE.Vector3(...data.min), new THREE.Vector3(...data.max));
  const sphere = box.getBoundingSphere(new THREE.Sphere());
  const geos = [], use = new Map();
  for (const grp of data.groups) for (const t of grp.textures || []) if (t) use.set(t, (use.get(t) || 0) + grp.index.length);
  const fallback = [...use].sort((a, b) => b[1] - a[1])[0]?.[0];
  for (const grp of data.groups) {
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', pos); geo.setAttribute('aN', aN); geo.setAttribute('aW0', aW0); geo.setAttribute('aW1', aW1);
    geo.setIndex(new THREE.BufferAttribute(grp.index, 1));
    geo.boundingBox = box; geo.boundingSphere = sphere;
    const m = new THREE.Mesh(geo, makeTerrainMaterial(grp, tex, env, fade, fallback));
    m.matrixAutoUpdate = false; m.frustumCulled = true; m.castShadow = true;
    g.add(m); geos.push(geo);
  }
  g.position.set(...data.translation); g.updateMatrixWorld(true);
  g.traverse(o => { if (o.isMesh) { o.matrixAutoUpdate = false; o.updateMatrix(); } });
  g.userData.dispose = () => { for (const geo of geos) geo.dispose(); g.traverse(o => o.material?.dispose()); };
  return g;
}
