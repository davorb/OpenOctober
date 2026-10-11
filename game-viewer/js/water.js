// Water: Gerstner-wave normals (the game's _GAmplitude/_GFinalFrequency/_GSteepness/_GFinalSpeed/_GDirection constants from roa_water.teq),
// the shipped water1_normal.dds detail normal scrolled along the wind, shallow/deep palette, Fresnel sky reflection and sun glitter.
import * as THREE from 'three';
import { DDSLoader } from 'three/addons/loaders/DDSLoader.js';
import { fogUniforms, fogFn } from './chunks.js';

export const SEA_LEVEL = 215;   // SFDarkfallConfig.DARKFALL_SEA_LEVEL

const vs = `
varying vec3 vWorld; varying vec3 vView;
void main() { vec4 wp = modelMatrix * vec4(position, 1.0); vWorld = wp.xyz; vec4 mv = viewMatrix * wp; vView = mv.xyz; gl_Position = projectionMatrix * mv; }`;

const fs = `
${fogUniforms}${fogFn}
uniform sampler2D tNormal; uniform float uTime; uniform float uFade; uniform vec3 uSunDir; uniform vec3 uAmbSky; uniform vec3 uZenith;
uniform sampler2D tDepth; uniform vec3 uRect;   // per-tile terrain height grid (x0, z0, has) -> real water depth
uniform vec3 uShallow; uniform vec3 uDeep; uniform vec3 uScatter; uniform float uWave; uniform float uOpacity; uniform float uKind; uniform vec3 uWind;
varying vec3 vWorld; varying vec3 vView;
// 4 Gerstner components, constants from the game's shader
const vec4 GAmp = vec4(0.05, 0.076, 0.063, 0.47);
const vec4 GFreq = vec4(0.08, 0.173, 0.0957, 0.31) * 1.5;
const vec4 GSteep = vec4(4.0, 3.0, 4.0, 3.0);
const vec4 GSpeed = vec4(-3.0, 2.34, 1.71, 3.37) * 0.45;
const vec4 GDirAB = vec4(0.3, 0.85, 0.85, 0.25);
const vec4 GDirCD = vec4(0.1, 0.9, 0.5, 0.5);
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float vnoise(vec2 p) { vec2 i = floor(p), f = fract(p); f = f * f * (3. - 2. * f);
  return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y); }
vec3 gerstnerNormal(vec2 p, float t) {
  p += (vec2(vnoise(p * 0.011), vnoise(p * 0.011 + 7.3)) - 0.5) * 26.0;   // break the straight crests up
  vec2 d0 = normalize(GDirAB.xy), d1 = normalize(GDirAB.zw), d2 = normalize(GDirCD.xy), d3 = normalize(GDirCD.zw);
  vec4 ph = vec4(dot(p, d0), dot(p, d1), dot(p, d2), dot(p, d3)) * GFreq * 6.2831853 / 6.0 + GSpeed * t;
  vec4 c = cos(ph) * GAmp * GFreq * 6.2831853 / 6.0;
  vec2 g = d0 * c.x + d1 * c.y + d2 * c.z + d3 * c.w;
  return normalize(vec3(-g.x * uWave * 1.1, 1.0, -g.y * uWave * 1.1));
}
void main() {
  vec3 V = normalize(-vView);                       // view space; bring normal to view space below
  vec2 wp = vWorld.xz;
  vec3 N = gerstnerNormal(wp, uTime);
  N = normalize(mix(N, vec3(0.0, 1.0, 0.0), smoothstep(150.0, 900.0, length(vView)) * 0.95));
  // scrolling detail normals (two layers + a far layer to break tiling)
  vec2 w2 = normalize(uWind.xz + vec2(0.001));
  vec2 uv1 = wp * 0.31 + w2 * uTime * 0.05, uv2 = wp.yx * 0.17 * vec2(1.0, -1.0) - w2.yx * uTime * 0.03, uv3 = wp * 0.037 + uTime * 0.006;
  vec2 n1 = texture2D(tNormal, uv1).ag * 2.0 - 1.0, n2 = texture2D(tNormal, uv2).ag * 2.0 - 1.0, n3 = texture2D(tNormal, uv3).ag * 2.0 - 1.0;
  float dist = length(vView);
  float nearF = 1.0 - smoothstep(60.0, 420.0, dist);
  vec2 dn = (n1 * 0.55 + n2 * 0.45) * 0.55 * nearF + n3 * 0.35;
  N = normalize(N + vec3(dn.x, 0.0, dn.y) * (uKind < 0.5 ? 1.0 : 0.6));
  mat3 viewRot = mat3(viewMatrix);
  vec3 Nv = normalize(viewRot * N);
  float ndv = clamp(dot(Nv, V), 0.0, 1.0);
  float fres = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);
  vec3 R = reflect(-V, Nv);                          // view space reflection
  vec3 Rw = normalize(transpose(viewRot) * R);
  float up = clamp(Rw.y, 0.0, 1.0);
  vec3 sky = mix(uFogColor, uZenith, pow(up, 0.5));
  // body colour: deeper looking down steeply, more turquoise scatter against the sun
  float depthK = smoothstep(0.0, 1.0, ndv);
  vec3 body = mix(uShallow, uDeep, depthK * 0.85);
  float sunSide = pow(max(dot(normalize(-vView), -normalize(mat3(viewMatrix) * uSunDir)), 0.0), 3.0);
  body += uScatter * (0.25 * sunSide * clamp(uSunDir.y + 0.2, 0.0, 1.0)) * (0.4 + 0.6 * (1.0 - ndv));
  body *= (uSunColor * 0.45 + uAmbSky) * 1.5 + 0.05;
  // real water depth from the terrain height grid (one fetch): colour absorption, shore fade, foam
  float d = 60.0;
  if (uRect.z > 0.5) d = vWorld.y - texture2D(tDepth, ((vWorld.xz - uRect.xy) * 0.25 + 0.5) / 257.0).r;
  float absorb = 1.0 - exp(-max(d, 0.0) * 0.16);
  body = mix(uShallow * 1.35 + uScatter * 0.15, body, absorb);
  float foam = 0.0;
  if (uKind < 1.5 && d < 4.0 && dist < 700.0) {
    float t = uTime;
    float wob = d + 0.35 * sin(t * 1.1 + vnoise(wp * 0.05) * 6.0) + 0.2 * sin(t * 2.3 + wp.x * 0.1);
    float band = 1.0 - smoothstep(0.0, 2.8, wob);
    float fn = vnoise(wp * 0.8 + t * 0.15) * 0.55 + vnoise(wp * 2.6 - t * 0.25) * 0.45;
    foam = clamp(smoothstep(0.38, 0.62, fn + band * 0.4 - 0.1) * band + (1.0 - smoothstep(0.0, 0.3, wob)) * 0.8, 0.0, 1.0);
    foam *= 1.0 - smoothstep(350.0, 700.0, dist);
  }
  vec3 col = mix(body, sky, fres);
  col = mix(col, (uSunColor * 0.5 + uAmbSky) * 1.4 + 0.12, foam * 0.9);
  vec3 Sv = normalize(viewRot * uSunDir);
  float spec = pow(max(dot(R, Sv), 0.0), 300.0) * 6.0 + pow(max(dot(R, Sv), 0.0), 40.0) * 0.25;
  col += uSunColor * spec * step(0.0, uSunDir.y + 0.05);
  float alpha = mix(uOpacity, 1.0, clamp(fres * 1.5 + smoothstep(100.0, 900.0, dist) * 0.5, 0.0, 1.0));
  alpha *= mix(0.25, 1.0, absorb) * smoothstep(-0.1, 0.5, d);   // see the bed through shallows, soft shoreline
  alpha = max(alpha, foam * 0.85 * smoothstep(-0.1, 0.15, d));
  if (uKind > 1.5) { // lava / acid style: emissive
    float f = vnoise(wp * 0.15 + uTime * 0.05) * 0.6 + vnoise(wp * 0.6 - uTime * 0.12) * 0.4;
    col = mix(col, uShallow * (1.2 + f * 2.0), 0.85); alpha = 1.0;
  }
  col = dfFog(col, vView, uFade);
  gl_FragColor = vec4(col, alpha * mix(0.0, 1.0, uFade));
  #include <colorspace_fragment>
}`;

const KINDS = {
  water: { shallow: [0.10, 0.34, 0.36], deep: [0.015, 0.08, 0.14], op: 0.72, wave: 0.14, kind: 0 },
  swamp: { shallow: [0.16, 0.22, 0.10], deep: [0.04, 0.07, 0.04], op: 0.92, wave: 0.2, kind: 1 },
  acid:  { shallow: [0.35, 0.9, 0.10], deep: [0.08, 0.25, 0.03], op: 0.95, wave: 0.3, kind: 2 },
  lava:  { shallow: [1.0, 0.32, 0.05], deep: [0.4, 0.05, 0.01], op: 1.0, wave: 0.1, kind: 2 },
  tar:   { shallow: [0.03, 0.03, 0.03], deep: [0.01, 0.01, 0.01], op: 0.98, wave: 0.15, kind: 1 },
};
export const kindOf = type => ({ r_Water: 'water', r_Swamp: 'swamp', r_Acid: 'acid', r_Lava: 'lava', r_Tar: 'tar' }[type] || 'water');

export class WaterFactory {
  constructor(env, zenith) {
    this.env = env; this.zenith = zenith; this.cache = new Map();
    this.blank = new THREE.DataTexture(new Float32Array(1), 1, 1, THREE.RedFormat, THREE.FloatType); this.blank.needsUpdate = true;
    this.normal = new DDSLoader().load('/dds/water1_normal.dds'); this.normal.wrapS = this.normal.wrapT = THREE.RepeatWrapping;
    this.normal.minFilter = THREE.LinearMipmapLinearFilter; this.normal.anisotropy = 4;
  }
  material(kindName, fade) {
    const k = KINDS[kindName] || KINDS.water;
    const m = new THREE.ShaderMaterial({
      vertexShader: vs, fragmentShader: fs, transparent: true, depthWrite: false, side: THREE.DoubleSide,
      uniforms: { ...this.env.u, tNormal: { value: this.normal }, uFade: fade || { value: 1 }, uZenith: this.zenith,
        uShallow: { value: new THREE.Color(...k.shallow) }, uDeep: { value: new THREE.Color(...k.deep) }, uScatter: { value: new THREE.Color(0.05, 0.45, 0.42) },
        tDepth: { value: this.blank }, uRect: { value: new THREE.Vector3() }, uWave: { value: k.wave }, uOpacity: { value: k.op }, uKind: { value: k.kind } },
    });
    m.userData.water = true; return m;
  }
  // give a water material the tile's terrain height grid (3 floats rect + R32F texture), so depth/foam work at shores
  bind(m, t) {
    if (!t || !t.heights) { (t.waterMats ||= []).push(m); return; }
    if (!t.waterTex) {
      const tx = new THREE.DataTexture(t.heights, 257, 257, THREE.RedFormat, THREE.FloatType);
      tx.minFilter = tx.magFilter = this.linearFloat ? THREE.LinearFilter : THREE.NearestFilter;
      tx.wrapS = tx.wrapT = THREE.ClampToEdgeWrapping; tx.needsUpdate = true; t.waterTex = tx;
    }
    m.uniforms.tDepth.value = t.waterTex; m.uniforms.uRect.value.set(t.rect.x0, t.rect.z0, 1);
  }
}
