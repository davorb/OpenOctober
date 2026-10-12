// Post-processing chain ported from the game's pp_*.teq passes:
//   scene (HDR, MSAA) -> pp_brightpass (+ pp_down4) -> pp_gaussianh/v -> pp_godray (half res) ->
//   pp_up4tonemapping (CombineToneMap2: extended Reinhard, white cutoff 0.8, middle grey 0.18) + vignette + pp_finalhdrl (scene + bloom) ->
//   pp_final_underwater (horizontal sine wobble, bloom x0.5) when the camera is below the sea.
// Not ported: pp_grade (64^3 colour LUTs are not in the extracted data), pp_fxaa (MSAA on the HDR target instead), SSAO.
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

const FINAL = `
uniform sampler2D tScene; uniform sampler2D tBloom; uniform sampler2D tGod;
uniform float uBloom, uExposure, uVignette, uUnder, uTime, uAspect; uniform vec3 uUnderTint;
varying vec2 vUv; ${LUMA}
void main() {
  vec2 uv = vUv;
  if (uUnder > 0.0) uv.x += sin(uv.y * 90.0 + uTime * 1.6) * 0.0018 * uUnder + sin(uv.y * 31.0 - uTime * 1.1) * 0.0012 * uUnder;   // pp_final_underwater: sine wobble
  vec3 c = texture2D(tScene, uv).rgb * uExposure;
  // CombineToneMap2: extended Reinhard (white cutoff 0.8)
  c *= 1.0 + c / (0.8 * 0.8); c /= 1.0 + c;
  c += texture2D(tBloom, uv).rgb * uBloom * mix(1.0, 0.5, uUnder);
  c += texture2D(tGod, uv).rgb;
  vec2 n = (vUv - 0.5) * vec2(uAspect > 1.0 ? 1.0 : uAspect, uAspect > 1.0 ? 1.0 / uAspect : 1.0);   // roughly circular
  float v = 1.0 - dot(n, n) * 1.15; c *= mix(1.0, v * v, uVignette);
  if (uUnder > 0.0) { float l = dot(c, LUMA); c = mix(c, uUnderTint * (l + 0.04) * 1.6, 0.55 * uUnder); }
  gl_FragColor = vec4(c, 1.0);
  #include <colorspace_fragment>
}`;

export class Post {
  constructor(renderer) {
    this.r = renderer; this.enabled = true;
    const hf = { type: THREE.HalfFloatType, minFilter: THREE.LinearFilter, magFilter: THREE.LinearFilter, depthBuffer: false };
    this.scene = new THREE.WebGLRenderTarget(4, 4, { ...hf, depthBuffer: true, samples: 4 });
    this.bright = new THREE.WebGLRenderTarget(4, 4, hf); this.blurA = new THREE.WebGLRenderTarget(4, 4, hf); this.blurB = new THREE.WebGLRenderTarget(4, 4, hf);
    this.god = new THREE.WebGLRenderTarget(4, 4, hf);
    const mk = (fragmentShader, uniforms) => new THREE.ShaderMaterial({ vertexShader: VS, fragmentShader, uniforms, depthTest: false, depthWrite: false });
    this.mBright = mk(BRIGHT, { tScene: { value: null }, uTexel: { value: new THREE.Vector2() }, uThreshold: { value: 0.8 }, uExposure: { value: 1 } });
    this.mBlur = mk(BLUR, { tSrc: { value: null }, uDir: { value: new THREE.Vector2() } });
    this.mGod = mk(GOD, { tScene: { value: null }, uParams: { value: new THREE.Vector4(0.5, 0.5, 0, 0.95) }, uColor: { value: new THREE.Color(1, 1, 1) }, uAspect: { value: 1 } });
    this.mFinal = mk(FINAL, { tScene: { value: null }, tBloom: { value: null }, tGod: { value: null }, uBloom: { value: 0.9 }, uExposure: { value: 1.3 }, uVignette: { value: 0.4 },
      uUnder: { value: 0 }, uTime: { value: 0 }, uAspect: { value: 1 }, uUnderTint: { value: new THREE.Color(0.25, 0.65, 0.7) } });
    this.quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2)); this.quad.frustumCulled = false;
    this.cam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1); this.qs = new THREE.Scene(); this.qs.add(this.quad);
    this.sunScreen = new THREE.Vector3();
  }
  resize(w, h) {
    this.w = w; this.h = h;
    this.scene.setSize(w, h);
    const q = [Math.max(2, w >> 2), Math.max(2, h >> 2)];
    this.bright.setSize(...q); this.blurA.setSize(...q); this.blurB.setSize(...q);
    this.god.setSize(Math.max(2, w >> 1), Math.max(2, h >> 1));
    this.mFinal.uniforms.uAspect.value = this.mGod.uniforms.uAspect.value = w / h;
  }
  pass(mat, target) { this.quad.material = mat; this.r.setRenderTarget(target); this.r.render(this.qs, this.cam); }
  // sun: world direction (three space), sunColor, daylight 0..1; underwater 0..1
  render(scene, camera, { sunDir, sunColor, daylight, under, time }) {
    const r = this.r;
    if (!this.enabled) { r.setRenderTarget(null); r.render(scene, camera); return; }
    r.setRenderTarget(this.scene); r.render(scene, camera);
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
    f.tScene.value = this.scene.texture; f.tBloom.value = src.texture; f.tGod.value = this.god.texture; f.uUnder.value = under; f.uTime.value = time;
    this.pass(this.mFinal, null);
  }
}
