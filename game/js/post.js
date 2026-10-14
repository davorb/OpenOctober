// Post-processing chain ported from the game's pp_*.teq passes:
//   scene (HDR, MSAA, depth texture) -> SSAO (half res, bilateral blur) -> pp_brightpass (+ pp_down4) -> pp_gaussianh/v -> pp_godray (half res) ->
//   pp_up4tonemapping (CombineToneMap2: extended Reinhard, white cutoff 0.8, middle grey 0.18) + vignette + pp_finalhdrl (scene + bloom) ->
//   pp_final_underwater (horizontal sine wobble, bloom x0.5) when the camera is below the sea.
//   -> pp_grade (zone "mood" LUT at strength 0.75, then the time-of-day LUT; 64^3 volume LUTs from export/texture/lut_*.dds) on the final LDR colour.
// Not ported: pp_grade's edge fade (it keeps silhouettes against bright backgrounds ungraded), pp_fxaa (MSAA on the HDR target instead).
import * as THREE from 'three';

const VS = `varying vec2 vUv; void main() { vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }`;
const LUMA = 'const vec3 LUMA = vec3(0.2125, 0.7154, 0.0721);';

// pp_brightpass: subtract threshold, then x/(offset+x) soft knee. Written on a 1/4-res target with a 4-tap box (pp_down4).
const BRIGHT = `
uniform sampler2D tScene; uniform vec2 uTexel; uniform float uThreshold; uniform float uExposure;
varying vec2 vUv; ${LUMA}
vec3 bp(vec3 c) { c *= uExposure; c = max(c - uThreshold, 0.0); return c / (0.6 + c); }
void main() {
  vec3 s = bp(texture2D(tScene, vUv + uTexel * vec2(-1.0, -1.0)).rgb) + bp(texture2D(tScene, vUv + uTexel * vec2(1.0, -1.0)).rgb)
         + bp(texture2D(tScene, vUv + uTexel * vec2(-1.0, 1.0)).rgb) + bp(texture2D(tScene, vUv + uTexel * vec2(1.0, 1.0)).rgb);
  gl_FragColor = vec4(s * 0.25, 1.0);
}`;

// pp_gaussianh / pp_gaussianv: 13-tap, sigma ~2
const BLUR = `
uniform sampler2D tSrc; uniform vec2 uDir; varying vec2 vUv;
void main() {
  float w[7]; w[0] = 0.176033; w[1] = 0.120985; w[2] = 0.064759; w[3] = 0.026995; w[4] = 0.008764; w[5] = 0.002216; w[6] = 0.0005;
  vec3 c = texture2D(tSrc, vUv).rgb * w[0];
  for (int i = 1; i < 7; i++) { vec2 o = uDir * float(i); c += (texture2D(tSrc, vUv + o).rgb + texture2D(tSrc, vUv - o).rgb) * w[i]; }
  gl_FragColor = vec4(c, 1.0);
}`;

// pp_godray: 32 radial taps toward the light, only bright pixels within SOURCE_RADIUS of it contribute, per-tap decay
const GOD = `
uniform sampler2D tScene; uniform vec4 uParams; uniform vec3 uColor; uniform float uAspect; varying vec2 vUv;
#define NUM_TAPS 32
#define SOURCE_RADIUS 0.5
void main() {
  vec2 toLight = uParams.xy - vUv; vec2 stepUV = toLight / float(NUM_TAPS); vec2 uv = vUv;
  float w = 1.0; vec3 acc = vec3(0.0);
  float md = length(toLight * vec2(uAspect, 1.0)) / SOURCE_RADIUS, mdStep = md / float(NUM_TAPS);
  for (int i = 0; i < NUM_TAPS; i++) {
    uv += stepUV; md -= mdStep;
    vec3 s = clamp(texture2D(tScene, uv).rgb - 0.35, 0.0, 4.0) * 1.55;
    float m = clamp(1.0 - md, 0.0, 1.0);
    acc += s * (w * m * m); w *= uParams.w;
  }
  gl_FragColor = vec4(acc / float(NUM_TAPS) * uParams.z * uColor, 1.0);
}`;

// SSAO (roa_ssao.frag calculateSSAOValue2 style: 16-point sample sphere, per-pixel random rotation from a 4x4 pattern, range check).
// Depth comes from the scene target; the normal is rebuilt from the depth neighbours. Output: r = occlusion (1 = open), g = linear depth for the blur.
const AO = `
uniform sampler2D tDepth; uniform mat4 uProj; uniform mat4 uProjInv; uniform vec2 uTexel; uniform float uNear, uFar, uRadius, uStrength; varying vec2 vUv;
float lin(float d) { float z = d * 2.0 - 1.0; return 2.0 * uNear * uFar / (uFar + uNear - z * (uFar - uNear)); }
vec3 viewPos(vec2 uv, float d) { vec4 p = uProjInv * vec4(uv * 2.0 - 1.0, d * 2.0 - 1.0, 1.0); return p.xyz / p.w; }
float hash(vec2 p) { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }
const vec3 K[16] = vec3[16](vec3(0.5381, 0.1856, 0.4319), vec3(0.1379, 0.2486, 0.4430), vec3(0.3371, 0.5679, 0.0057), vec3(-0.6999, -0.0451, 0.0019),
  vec3(0.0689, -0.1598, 0.8547), vec3(0.0560, 0.0069, 0.1843), vec3(-0.0146, 0.1402, 0.0762), vec3(0.0100, -0.1924, 0.0344),
  vec3(-0.3577, -0.5301, 0.4358), vec3(-0.3169, 0.1063, 0.0158), vec3(0.0103, -0.5869, 0.0046), vec3(-0.0897, -0.4940, 0.3287),
  vec3(0.7119, -0.0154, 0.0918), vec3(-0.0533, 0.0596, 0.5411), vec3(0.0352, -0.0631, 0.5460), vec3(-0.4776, 0.2847, 0.0271));
void main() {
  float d = texture2D(tDepth, vUv).r;
  if (d >= 0.99999) { gl_FragColor = vec4(1.0, uFar, 0.0, 1.0); return; }
  vec3 P = viewPos(vUv, d); float dist = -P.z;
  float fade = 1.0 - smoothstep(60.0, 260.0, dist);
  if (fade <= 0.0) { gl_FragColor = vec4(1.0, dist, 0.0, 1.0); return; }
  // normal from the smaller depth difference per axis
  vec2 e = uTexel * 2.0;
  vec3 pr = viewPos(vUv + vec2(e.x, 0.0), texture2D(tDepth, vUv + vec2(e.x, 0.0)).r), pl = viewPos(vUv - vec2(e.x, 0.0), texture2D(tDepth, vUv - vec2(e.x, 0.0)).r);
  vec3 pu = viewPos(vUv + vec2(0.0, e.y), texture2D(tDepth, vUv + vec2(0.0, e.y)).r), pd = viewPos(vUv - vec2(0.0, e.y), texture2D(tDepth, vUv - vec2(0.0, e.y)).r);
  vec3 dx = abs(pr.z - P.z) < abs(P.z - pl.z) ? pr - P : P - pl, dy = abs(pu.z - P.z) < abs(P.z - pd.z) ? pu - P : P - pd;
  vec3 N = normalize(cross(dx, dy)); if (dot(N, P) > 0.0) N = -N;
  // random rotation about the normal
  float a = hash(gl_FragCoord.xy) * 6.2831853;
  vec3 rv = vec3(cos(a), sin(a), 0.0); vec3 T = normalize(rv - N * dot(rv, N)); if (abs(dot(rv, N)) > 0.99) T = normalize(cross(N, vec3(0.0, 1.0, 0.0))); vec3 B = cross(N, T);
  float occ = 0.0;
  for (int i = 0; i < 16; i++) {
    vec3 k = K[i]; float sc = 0.25 + 0.75 * float(i) / 15.0;   // more samples near the origin
    vec3 S = P + (T * k.x + B * k.y + N * k.z) * (uRadius * sc * 1.6);
    vec4 pc = uProj * vec4(S, 1.0); vec2 uv = pc.xy / pc.w * 0.5 + 0.5;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) continue;
    float sz = lin(texture2D(tDepth, uv).r);                      // scene distance along the view ray at the sample
    float range = smoothstep(0.0, 1.0, uRadius / max(abs(dist - sz), 1e-3));
    occ += (sz < -S.z - 0.08 * uRadius ? 1.0 : 0.0) * range;
  }
  float ao = 1.0 - occ / 16.0 * 2.2 * uStrength * fade;
  gl_FragColor = vec4(clamp(ao, 0.0, 1.0), dist, 0.0, 1.0);
}`;

// depth-aware separable blur of the AO (stands in for the game's 4x4 blur)
const AOBLUR = `
uniform sampler2D tAO; uniform vec2 uDir; varying vec2 vUv;
void main() {
  vec2 c = texture2D(tAO, vUv).rg; float sum = c.x, wsum = 1.0;
  for (int i = 1; i <= 4; i++) for (int s = -1; s <= 1; s += 2) {
    vec2 t = texture2D(tAO, vUv + uDir * float(i * s)).rg; float w = (1.0 - float(i) / 5.0) * exp(-abs(t.y - c.y) / max(c.y * 0.02, 0.15));
    sum += t.x * w; wsum += w;
  }
  gl_FragColor = vec4(sum / wsum, c.y, 0.0, 1.0);
}`;

const FINAL = `
uniform sampler2D tScene; uniform sampler2D tBloom; uniform sampler2D tGod; uniform sampler2D tAO;
uniform float uBloom, uExposure, uVignette, uUnder, uTime, uAspect; uniform vec3 uUnderTint;
uniform highp sampler3D tMoodA, tMoodB, tTodA, tTodB; uniform float uMoodBlend, uMoodStrength, uTodBlend, uTodStrength;
varying vec2 vUv; ${LUMA}
void main() {
  vec2 uv = vUv;
  if (uUnder > 0.0) uv.x += sin(uv.y * 90.0 + uTime * 1.6) * 0.0018 * uUnder + sin(uv.y * 31.0 - uTime * 1.1) * 0.0012 * uUnder;   // pp_final_underwater: sine wobble
  vec3 c = texture2D(tScene, uv).rgb * uExposure * texture2D(tAO, uv).r;
  // CombineToneMap2: extended Reinhard (white cutoff 0.8)
  c *= 1.0 + c / (0.8 * 0.8); c /= 1.0 + c;
  c += texture2D(tBloom, uv).rgb * uBloom * mix(1.0, 0.5, uUnder);
  c += texture2D(tGod, uv).rgb;
  vec2 n = (vUv - 0.5) * vec2(uAspect > 1.0 ? 1.0 : uAspect, uAspect > 1.0 ? 1.0 / uAspect : 1.0);   // roughly circular
  float v = 1.0 - dot(n, n) * 1.15; c *= mix(1.0, v * v, uVignette);
  if (uUnder > 0.0) { float l = dot(c, LUMA); c = mix(c, uUnderTint * (l + 0.04) * 1.6, 0.55 * uUnder); }
  gl_FragColor = vec4(c, 1.0);
  #include <colorspace_fragment>
  // pp_grade: the LUTs map the display-encoded LDR frame; mood (zone) stage, then the time-of-day stage on top of it
  vec3 g = gl_FragColor.rgb;
  if (uMoodStrength > 0.0) { vec3 q = g * (63.0 / 64.0) + 0.5 / 64.0; g = mix(g, mix(texture(tMoodA, q).rgb, texture(tMoodB, q).rgb, uMoodBlend), uMoodStrength); }
  if (uTodStrength > 0.0) { vec3 q = g * (63.0 / 64.0) + 0.5 / 64.0; g = mix(g, mix(texture(tTodA, q).rgb, texture(tTodB, q).rgb, uTodBlend), uTodStrength); }
  gl_FragColor.rgb = g;
}`;

// 64^3 BGRA8 volume LUTs (DDS, 128 byte header); cached by name. Resolves null if the file is missing.
const LUTS = new Map();
function loadLUT(name) {
  if (!LUTS.has(name)) LUTS.set(name, fetch(`/tex/lut_${name}.dds`).then(r => r.ok ? r.arrayBuffer() : null).then(buf => {
    if (!buf || buf.byteLength < 128 + 64 * 64 * 64 * 4) return null;
    const bgra = new Uint8Array(buf, 128, 64 * 64 * 64 * 4), rgba = new Uint8Array(bgra.length);
    for (let i = 0; i < bgra.length; i += 4) { rgba[i] = bgra[i + 2]; rgba[i + 1] = bgra[i + 1]; rgba[i + 2] = bgra[i]; rgba[i + 3] = 255; }
    const t = new THREE.Data3DTexture(rgba, 64, 64, 64);
    t.format = THREE.RGBAFormat; t.type = THREE.UnsignedByteType; t.minFilter = t.magFilter = THREE.LinearFilter;
    t.wrapS = t.wrapT = t.wrapR = THREE.ClampToEdgeWrapping; t.unpackAlignment = 1; t.needsUpdate = true;
    return t;
  }).catch(() => null));
  return LUTS.get(name);
}

export class Post {
  constructor(renderer) {
    this.r = renderer; this.enabled = true;
    const hf = { type: THREE.HalfFloatType, minFilter: THREE.LinearFilter, magFilter: THREE.LinearFilter, depthBuffer: false };
    this.scene = new THREE.WebGLRenderTarget(4, 4, { ...hf, depthBuffer: true, samples: 4 }); this.scene.depthTexture = new THREE.DepthTexture(4, 4);
    this.ao = new THREE.WebGLRenderTarget(4, 4, hf); this.aoB = new THREE.WebGLRenderTarget(4, 4, hf); this.ssao = true;
    this.bright = new THREE.WebGLRenderTarget(4, 4, hf); this.blurA = new THREE.WebGLRenderTarget(4, 4, hf); this.blurB = new THREE.WebGLRenderTarget(4, 4, hf);
    this.god = new THREE.WebGLRenderTarget(4, 4, hf);
    const mk = (fragmentShader, uniforms) => new THREE.ShaderMaterial({ vertexShader: VS, fragmentShader, uniforms, depthTest: false, depthWrite: false });
    this.mBright = mk(BRIGHT, { tScene: { value: null }, uTexel: { value: new THREE.Vector2() }, uThreshold: { value: 0.8 }, uExposure: { value: 1 } });
    this.mBlur = mk(BLUR, { tSrc: { value: null }, uDir: { value: new THREE.Vector2() } });
    this.mGod = mk(GOD, { tScene: { value: null }, uParams: { value: new THREE.Vector4(0.5, 0.5, 0, 0.95) }, uColor: { value: new THREE.Color(1, 1, 1) }, uAspect: { value: 1 } });
    this.mAO = mk(AO, { tDepth: { value: null }, uProj: { value: new THREE.Matrix4() }, uProjInv: { value: new THREE.Matrix4() }, uTexel: { value: new THREE.Vector2() }, uNear: { value: 1 }, uFar: { value: 1000 }, uRadius: { value: 3.0 }, uStrength: { value: 1 } });
    this.mAOBlur = mk(AOBLUR, { tAO: { value: null }, uDir: { value: new THREE.Vector2() } });
    this.white = new THREE.DataTexture(new Uint8Array([255, 255, 255, 255]), 1, 1); this.white.needsUpdate = true;
    this.blank3 = new THREE.Data3DTexture(new Uint8Array([0, 0, 0, 255]), 1, 1, 1); this.blank3.format = THREE.RGBAFormat; this.blank3.type = THREE.UnsignedByteType; this.blank3.needsUpdate = true;
    this.grade = true;
    this.mFinal = mk(FINAL, { tScene: { value: null }, tBloom: { value: null }, tGod: { value: null }, tAO: { value: null }, uBloom: { value: 0.9 }, uExposure: { value: 1.3 }, uVignette: { value: 0.4 },
      uUnder: { value: 0 }, uTime: { value: 0 }, uAspect: { value: 1 }, uUnderTint: { value: new THREE.Color(0.25, 0.65, 0.7) },
      tMoodA: { value: this.blank3 }, tMoodB: { value: this.blank3 }, tTodA: { value: this.blank3 }, tTodB: { value: this.blank3 },
      uMoodBlend: { value: 0 }, uMoodStrength: { value: 0 }, uTodBlend: { value: 0 }, uTodStrength: { value: 0 } });
    this.quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2)); this.quad.frustumCulled = false;
    this.cam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1); this.qs = new THREE.Scene(); this.qs.add(this.quad);
    this.sunScreen = new THREE.Vector3();
  }
  resize(w, h) {
    this.w = w; this.h = h;
    this.scene.setSize(w, h);
    const q = [Math.max(2, w >> 2), Math.max(2, h >> 2)];
    this.bright.setSize(...q); this.blurA.setSize(...q); this.blurB.setSize(...q);
    this.ao.setSize(Math.max(2, w >> 1), Math.max(2, h >> 1)); this.aoB.setSize(Math.max(2, w >> 1), Math.max(2, h >> 1));
    this.god.setSize(Math.max(2, w >> 1), Math.max(2, h >> 1));
    this.mFinal.uniforms.uAspect.value = this.mGod.uniforms.uAspect.value = w / h;
  }
  // mood: [nameA, nameB, blend], tod: [nameA, nameB, blend] (LUT names without the lut_ prefix). The game's mood strength is 0.75; the tod stage runs at 1.
  setGrade(mood, tod) {
    const f = this.mFinal.uniforms, on = this.grade && mood && tod;
    const names = this._gn ||= {};
    for (const [key, name] of [['tMoodA', on && mood[0]], ['tMoodB', on && mood[1]], ['tTodA', on && tod[0]], ['tTodB', on && tod[1]]]) {
      if (names[key] === name) continue;
      names[key] = name;
      if (!name) { f[key].value = this.blank3; continue; }
      loadLUT(name).then(t => { if (t && names[key] === name) f[key].value = t; });
    }
    f.uMoodBlend.value = on ? mood[2] : 0; f.uTodBlend.value = on ? tod[2] : 0;
    const ready = k => f[k].value !== this.blank3;
    f.uMoodStrength.value = on && ready('tMoodA') && ready('tMoodB') ? 0.75 : 0;
    f.uTodStrength.value = on && ready('tTodA') && ready('tTodB') ? 1 : 0;
  }
  pass(mat, target) { this.quad.material = mat; this.r.setRenderTarget(target); this.r.render(this.qs, this.cam); }
  // sun: world direction (three space), sunColor, daylight 0..1; underwater 0..1
  render(scene, camera, { sunDir, sunColor, daylight, under, time }) {
    const r = this.r;
    const ov = this.overlay, over = () => { if (ov?.visible) { r.autoClear = false; r.clearDepth(); r.render(ov.scene, ov.cam); r.autoClear = true; } };   // first-person weapon (js/viewmodel.js) over the world
    if (!this.enabled) { r.setRenderTarget(null); r.render(scene, camera); over(); return; }
    r.setRenderTarget(this.scene); r.render(scene, camera); over();
    // SSAO at half res + depth-aware blur
    let aoTex = this.white;
    if (this.ssao) {
      const a = this.mAO.uniforms; a.tDepth.value = this.scene.depthTexture; a.uProj.value.copy(camera.projectionMatrix); a.uProjInv.value.copy(camera.projectionMatrixInverse);
      a.uTexel.value.set(1 / this.w, 1 / this.h); a.uNear.value = camera.near; a.uFar.value = camera.far;
      this.pass(this.mAO, this.ao);
      const b = this.mAOBlur.uniforms, aw = this.ao.width, ah = this.ao.height;
      b.tAO.value = this.ao.texture; b.uDir.value.set(1 / aw, 0); this.pass(this.mAOBlur, this.aoB);
      b.tAO.value = this.aoB.texture; b.uDir.value.set(0, 1 / ah); this.pass(this.mAOBlur, this.ao);
      aoTex = this.ao.texture;
    }
    // bloom
    this.mBright.uniforms.tScene.value = this.scene.texture; this.mBright.uniforms.uTexel.value.set(1 / this.w, 1 / this.h);
    this.pass(this.mBright, this.bright);
    const u = this.mBlur.uniforms, bw = this.bright.width, bh = this.bright.height;
    let src = this.bright;
    for (let i = 0; i < 2; i++) {
      u.tSrc.value = src.texture; u.uDir.value.set(1.5 / bw, 0); this.pass(this.mBlur, this.blurA);
      u.tSrc.value = this.blurA.texture; u.uDir.value.set(0, 1.5 / bh); this.pass(this.mBlur, this.blurB);
      src = this.blurB;
    }
    // god rays: sun in front of the camera, above the horizon, by day
    const s = this.sunScreen.copy(sunDir).multiplyScalar(100).add(camera.position).project(camera);
    const fwd = camera.getWorldDirection(this._f ||= new THREE.Vector3()).dot(sunDir);
    const gp = this.mGod.uniforms;
    const vis = Math.max(0, Math.min(1, fwd * 3 - 0.2)) * Math.max(0, Math.min(1, (sunDir.y + 0.02) * 12)) * Math.max(0, Math.min(1, (daylight - 0.1) * 2)) * (1 - under);
    if (vis > 0.01) {
      gp.tScene.value = this.scene.texture; gp.uParams.value.set(s.x * 0.5 + 0.5, s.y * 0.5 + 0.5, 0.75 * vis, 0.95);
      const m = Math.max(sunColor.r, sunColor.g, sunColor.b, 1e-3); gp.uColor.value.setRGB(sunColor.r / m, sunColor.g / m, sunColor.b / m).lerp(new THREE.Color(1, 1, 1), 0.25);
      this.pass(this.mGod, this.god);
    } else { gp.uParams.value.z = 0; gp.tScene.value = this.scene.texture; this.mGod.uniforms.uParams.value.set(0.5, 0.5, 0, 0.5); this.pass(this.mGod, this.god); }
    // composite to screen (sRGB encode happens in the shader)
    const f = this.mFinal.uniforms;
    f.tScene.value = this.scene.texture; f.tBloom.value = src.texture; f.tGod.value = this.god.texture; f.tAO.value = aoTex; f.uUnder.value = under; f.uTime.value = time;
    this.pass(this.mFinal, null);
  }
}
