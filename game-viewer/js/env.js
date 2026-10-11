// Time-of-day environment, driven by the game's own EnvironmentConfig / objectspace/environment keyframes (data/env.json).
// Each config is a list of (keyframe file, StartTime in minutes, InterpolationTime); at StartTime the world blends from the previous
// keyframe to this one over InterpolationTime minutes. A keyframe holds the planets (sun + moons: light colour, intensity, orbit rotation),
// atmosphere (ambient light, cloud colour/cover), and the global linear fog (colour, start/end as fraction of view distance).
import * as THREE from 'three';

const lerp = (a, b, k) => a + (b - a) * k;
const lerpArr = (a, b, k) => a && b ? a.map((v, i) => lerp(v, b[i], k)) : (b || a);
const smooth = k => k * k * (3 - 2 * k);

// D3DX yaw/pitch/roll applied to the planet's (0, distance, 0) offset (row-vector convention), game space (left handed, +Y up).
export function planetDir(rot, dist) {
  let [x, y, z] = dist || [0, 1, 0];
  const [pitch, yaw, roll] = rot || [0, 0, 0];
  let c = Math.cos(roll), s = Math.sin(roll);  [x, y] = [x * c - y * s, x * s + y * c];
  c = Math.cos(pitch); s = Math.sin(pitch);    [y, z] = [y * c - z * s, y * s + z * c];
  c = Math.cos(yaw); s = Math.sin(yaw);        [x, z] = [x * c + z * s, -x * s + z * c];
  const l = Math.hypot(x, y, z) || 1;
  return [x / l, y / l, -z / l];            // game z -> three z
}

const srgb = a => new THREE.Color().setRGB(a[0], a[1], a[2], THREE.SRGBColorSpace);

export class Environment {
  constructor(data) {
    this.data = data;
    this.names = Object.keys(data.configs).filter(n => data.configs[n].length > 4);
    this.config = 'human_clearsky';
    this.time = 12 * 60;          // minutes of day
    this.speed = 1;               // game minutes per real second
    this.auto = true;
    // shared uniforms (referenced by every custom material)
    this.u = {
      uSunDir: { value: new THREE.Vector3(0, 1, 0) }, uSunDirView: { value: new THREE.Vector3(0, 1, 0) }, uSunColor: { value: new THREE.Color() },
      uMoonDir: { value: [new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3()] }, uMoonColor: { value: [new THREE.Color(), new THREE.Color(), new THREE.Color()] },
      uAmbSky: { value: new THREE.Color() }, uAmbGround: { value: new THREE.Color() },
      uFogColor: { value: new THREE.Color() }, uFogRange: { value: new THREE.Vector2(1000, 3000) }, uFogDensity: { value: 1 },
      uTime: { value: 0 }, uViewDist: { value: 3000 },
      uWind: { value: new THREE.Vector3(0, 0, 0) },
    };
    this.state = {};
  }
  setConfig(n) { this.config = n; }
  cycleConfig() { this.config = this.names[(this.names.indexOf(this.config) + 1) % this.names.length]; return this.config; }
  hhmm() { const m = Math.floor(this.time) % 1440; return String(Math.floor(m / 60)).padStart(2, '0') + ':' + String(m % 60).padStart(2, '0'); }
  // music slot, boundaries follow the clear-sky keyframes (dawn 6:00 -> noon -> warm 15:00 -> dusk 18:40 -> night 22:00)
  slot() { const t = this.time % 1440; return t >= 300 && t < 660 ? 'sunrise' : t >= 660 && t < 900 ? 'midday' : t >= 900 && t < 1200 ? 'sunset' : 'midnight'; }

  frame(file) { return this.data.frames[file]; }
  blend() {
    const list = [...this.data.configs[this.config]].sort((a, b) => a.start - b.start);
    const t = this.time % 1440;
    let i = list.length - 1;
    for (let j = 0; j < list.length; j++) if (list[j].start <= t) i = j;
    const cur = list[i], prev = list[(i - 1 + list.length) % list.length];
    let dt = t - cur.start; if (dt < 0) dt += 1440;
    const k = smooth(Math.min(1, dt / Math.max(1, cur.interp)));
    return [this.frame(prev.file), this.frame(cur.file), k];
  }
  update(dt, viewDist) {
    if (this.auto) this.time = (this.time + dt * this.speed) % 1440;
    const [A, B, k] = this.blend();
    const s = this.state, u = this.u;
    // planets, matched by their xml
    const planets = {};
    for (const pb of B.planets) {
      const pa = A.planets.find(p => p.xml === pb.xml) || pb;
      planets[pb.xml] = {
        dir: lerpArr(planetDir(pa.rot, pa.dist), planetDir(pb.rot, pb.dist), k),
        color: lerpArr(pa.color, pb.color, k), intensity: lerp(pa.intensity ?? 0, pb.intensity ?? 0, k), visible: pb.visible,
      };
    }
    s.planets = planets;
    const sun = planets['planets/sun.xml'];
    u.uSunDir.value.set(...sun.dir).normalize();
    u.uSunColor.value.copy(srgb(sun.color)).multiplyScalar(sun.intensity);
    const moons = ['planets/blue_moon.xml', 'planets/ghost_moon.xml', 'planets/red_moon.xml'];
    moons.forEach((m, n) => { const p = planets[m]; if (!p) return; u.uMoonDir.value[n].set(...p.dir).normalize(); u.uMoonColor.value[n].copy(srgb(p.color)).multiplyScalar(p.intensity * 0.5); });
    const amb = lerpArr(A.ambient, B.ambient, k);
    s.ambient = amb;
    u.uAmbSky.value.copy(srgb(amb)).multiplyScalar(1.5);
    u.uAmbGround.value.copy(srgb(amb)).multiplyScalar(0.7).multiply(new THREE.Color(0.8, 0.85, 0.7));
    const fog = lerpArr(A.fogColor, B.fogColor, k);
    s.fogRaw = fog;
    u.uFogColor.value.copy(srgb(fog));
    const fs = lerp(A.fogStart, B.fogStart, k), fe = lerp(A.fogEnd, B.fogEnd, k);
    u.uFogRange.value.set(fs * viewDist, fe * viewDist);
    u.uFogDensity.value = lerp(A.fogDensity ?? 1, B.fogDensity ?? 1, k);
    u.uViewDist.value = viewDist;
    s.cloudColor = lerpArr(A.cloudColor, B.cloudColor, k);
    s.cloudCover = lerp(A.cloudCover ?? 0.3, B.cloudCover ?? 0.3, k);
    s.cloudThickness = lerp(A.cloudThickness ?? 0.5, B.cloudThickness ?? 0.5, k);
    s.wind = lerpArr(A.wind, B.wind, k);
    s.windStrength = lerp(A.windStrength ?? 0.3, B.windStrength ?? 0.3, k);
    if (s.wind) u.uWind.value.set(s.wind[0], 0, -s.wind[2]);
    // how bright the day is (for stars, sun disc, music): luminance of sun light vs. its height
    const lum = u.uSunColor.value.r * 0.3 + u.uSunColor.value.g * 0.59 + u.uSunColor.value.b * 0.11;
    s.daylight = Math.max(0, Math.min(1, (lum - 0.12) / 0.45)) * Math.max(0, Math.min(1, u.uSunDir.value.y * 4 + 0.3));
    u.uTime.value += dt;
  }
}
