// Sky dome (gradient matched to the fog colour, procedural clouds driven by the keyframe cloud cover/colour/wind),
// stars, sun disc and the three moons (blue / ghost / red) placed from the planet orbit data.
import * as THREE from 'three';

const vs = `
varying vec3 vDir;
void main() { vDir = position; vec4 p = projectionMatrix * mat4(mat3(viewMatrix)) * vec4(position, 1.0); gl_Position = p.xyww; }`;
const fs = `
uniform vec3 uFogColor; uniform vec3 uZenith; uniform vec3 uSunDir; uniform vec3 uSunColor; uniform vec3 uCloudColor;
uniform float uCover; uniform float uThick; uniform float uTime; uniform float uDay; uniform vec3 uWind;
varying vec3 vDir;
float hash(vec2 p) { p = fract(p * vec2(123.34, 456.21)); p += dot(p, p + 45.32); return fract(p.x * p.y); }
float noise(vec2 p) { vec2 i = floor(p), f = fract(p); f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y); }
float fbm(vec2 p) { float a = 0.5, s = 0.0; for (int i = 0; i < 5; i++) { s += a * noise(p); p = p * 2.03 + 17.1; a *= 0.5; } return s; }
void main() {
  vec3 d = normalize(vDir);
  float h = d.y;
  vec3 col = mix(uFogColor, uZenith, smoothstep(0.0, 1.0, pow(clamp(h, 0.0, 1.0), 0.5)));
  if (h < 0.0) col = mix(uFogColor, uFogColor * 0.85, clamp(-h * 4.0, 0.0, 1.0));
  float sd = max(dot(d, uSunDir), 0.0);
  col += uSunColor * (pow(sd, 6.0) * 0.12 + pow(sd, 48.0) * 0.28) * smoothstep(-0.25, 0.1, uSunDir.y);
  // stars
  vec2 sp = d.xz / (abs(d.y) + 0.6) * 90.0;
  float st = hash(floor(sp)); float twinkle = 0.6 + 0.4 * sin(uTime * 2.0 + st * 40.0);
  vec2 sf = fract(sp) - 0.5;
  float star = step(0.9925, st) * twinkle * smoothstep(0.02, 0.3, h) * smoothstep(0.16, 0.0, length(sf + (vec2(hash(floor(sp) + 3.1), hash(floor(sp) + 7.7)) - 0.5) * 0.5));
  col += vec3(star) * (1.0 - uDay) * 0.9;
  // clouds on a plane overhead
  if (h > 0.0) {
    vec2 uv = d.xz / (h + 0.18) * 0.55 + uWind.xz * uTime * 0.012;
    float n = fbm(uv) * 0.75 + fbm(uv * 3.1 + 5.0) * 0.25;
    float cov = clamp(uCover * 1.5 + 0.12, 0.0, 1.0);
    float c = smoothstep(1.0 - cov, 1.0 - cov + 0.28, n);
    float edge = smoothstep(0.0, 0.22, h);
    vec3 cc = uCloudColor * (0.55 + 0.6 * clamp(uSunDir.y + 0.35, 0.0, 1.0)) + uSunColor * pow(sd, 3.0) * 0.35;
    col = mix(col, mix(col, cc, 0.9), c * edge * clamp(0.35 + uThick * 0.7, 0.0, 1.0));
  }
  gl_FragColor = vec4(col, 1.0);
  #include <colorspace_fragment>
}`;

export class Sky {
  constructor(env, loader) {
    this.env = env;
    this.group = new THREE.Group();
    const u = this.uniforms = {
      uFogColor: env.u.uFogColor, uSunDir: env.u.uSunDir, uSunColor: env.u.uSunColor, uZenith: { value: new THREE.Color() }, uCloudColor: { value: new THREE.Color() },
      uCover: { value: 0.3 }, uThick: { value: 0.5 }, uTime: env.u.uTime, uDay: { value: 1 }, uWind: env.u.uWind,
    };
    const mat = new THREE.ShaderMaterial({ vertexShader: vs, fragmentShader: fs, uniforms: u, side: THREE.BackSide, depthWrite: false, depthTest: true, fog: false });
    this.dome = new THREE.Mesh(new THREE.SphereGeometry(1, 48, 24), mat);
    this.dome.frustumCulled = false; this.dome.renderOrder = -10; this.dome.scale.setScalar(900);
    this.group.add(this.dome);
    const sprite = (url, blending = THREE.AdditiveBlending) => {
      const t = loader.load(url); t.colorSpace = THREE.SRGBColorSpace;
      const m = new THREE.SpriteMaterial({ map: t, blending, depthWrite: false, depthTest: false, transparent: true, fog: false });
      const s = new THREE.Sprite(m); s.renderOrder = -9; s.frustumCulled = false; this.group.add(s); return s;
    };
    this.sunSprite = sprite('/converted/textures/sun_disc_01.png');
    const c = document.createElement('canvas'); c.width = c.height = 128; const g = c.getContext('2d'); const gr = g.createRadialGradient(64, 64, 0, 64, 64, 62);
    gr.addColorStop(0, '#fff'); gr.addColorStop(0.86, '#fff'); gr.addColorStop(1, '#000'); g.fillStyle = gr; g.fillRect(0, 0, 128, 128);
    this.disc = new THREE.CanvasTexture(c);
    this.moons = ['moon_blue2', 'moon_ghost', 'moon_red2'].map(n => { const sp = sprite(`/converted/textures/${n}.png`, THREE.NormalBlending); sp.material.alphaMap = this.disc; return sp; });
  }
  update(camera) {
    const e = this.env, s = e.state, u = this.uniforms;
    const fog = e.u.uFogColor.value, amb = e.u.uAmbSky.value;
    u.uZenith.value.setRGB(fog.r * 0.45 + amb.r * 0.45, fog.g * 0.55 + amb.g * 0.45, fog.b * 0.85 + amb.b * 0.45);
    u.uCloudColor.value.setRGB(...(s.cloudColor || [1, 1, 1]), THREE.SRGBColorSpace);
    u.uCover.value = s.cloudCover; u.uThick.value = s.cloudThickness; u.uDay.value = s.daylight;
    const R = camera.far * 0.45;
    this.dome.scale.setScalar(R);
    this.group.position.copy(camera.position);
    const place = (spr, dir, size, op) => { spr.position.copy(dir).multiplyScalar(R * 0.96); spr.scale.setScalar(size * R); spr.material.opacity = op; spr.visible = op > 0.01; };
    const sd = e.u.uSunDir.value;
    place(this.sunSprite, sd, 0.14, Math.min(1, Math.max(0, sd.y * 6 + 0.6)) * (0.4 + 0.6 * s.daylight));
    this.sunSprite.material.color.copy(e.u.uSunColor.value).multiplyScalar(1.0).addScalar(0.15);
    this.moons.forEach((m, i) => {
      const d = e.u.uMoonDir.value[i];
      const above = Math.min(1, Math.max(0, d.y * 5 + 0.5));
      place(m, d, [0.08, 0.06, 0.055][i], above * (1 - s.daylight) * 0.95);
    });
  }
}
