// Time-of-day environment, driven by the game's own EnvironmentConfig / objectspace/environment keyframes (data/env.json).
// Each config is a list of (keyframe file, StartTime in minutes, InterpolationTime); at StartTime the world blends from the previous
// keyframe to this one over InterpolationTime minutes. A keyframe holds the planets (sun + moons: light colour, intensity, orbit rotation),
// atmosphere (ambient light, cloud colour/cover), and the global linear fog (colour, start/end as fraction of view distance).
import * as THREE from 'three';
import { addLightUniforms } from './lights.js';

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

// Regional atmosphere. The client data has no region->sky table (the server drives it), but it ships designer keyframes authored on tiles inside each
// region (d28_desert, e08/h18/j12/o08/o21/s20/t15 'light_henning', ...), race/winter/ork settings and a complete volcano day cycle. Each music-area region
// (same tile partition as the music) maps to those; their palette relative to the reference human noon is applied as a grade over the time-of-day cycle.
const REGION_FRAMES = {
  'Desert Continent': ['d28_desert.xml'], 'Ork Land': ['t15_yellow_light.xml', 'v12_light_henning.xml'], 'Alfar Land': ['o21_light_henning.xml'],
  'Human Land': ['s20_light_henning.xml'], 'Mirdain Land': ['h18_light_henning.xml'], 'Mahirim Land': ['j12_light_henning.xml'],
  'Dwarven Land': ['o08_light_henning.xml', 's08_day.xml'], 'Ice continent': ['e08_light_henning.xml', 'env_setting_winter_01.xml'],
  'The Frostheld': ['env_setting_winter_01.xml'], 'Jungle Continent': ['cc09_light_henning.xml'], 'Island Area': ['b22_demotile_setting.xml'],
  'Wasteland Continent': [{ config: 'volcano_clear' }],
};
for (const [a, b] of [['Ork Starting Area', 'Ork Land'], ['Alfar Starting Area', 'Alfar Land'], ['Human Starting Area', 'Human Land'], ['Mirdain Starting Area', 'Mirdain Land'],
  ['Mahirim Starting Area', 'Mahirim Land'], ['Dwarf Starting Area', 'Dwarven Land']]) REGION_FRAMES[a] = REGION_FRAMES[b];
const REF_FRAME = 'env_human_clear01_a.xml';       // noon of the default clear-sky cycle
const clampR = v => Math.max(0.4, Math.min(2.4, v));

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
    this.u.uShadowMap = { value: null }; this.u.uShadowMat = { value: new THREE.Matrix4() }; this.u.uShadowOn = { value: 0 }; this.u.uShadowTexel = { value: 1 / 2048 };
    addLightUniforms(this.u);
    this.state = {};
    this.regionW = new Map(); this.regionT = new Map();   // area name -> weight (smoothed / target)
    this.region = '';
  }
  // target weights from the player's surroundings (music-area names), smoothed in update()
  setRegionTarget(map) { this.regionT = map; if (!this.regionW.size) this.regionW = new Map(map); }
  setConfig(n) { this.config = n; }
  cycleConfig() { this.config = this.names[(this.names.indexOf(this.config) + 1) % this.names.length]; return this.config; }
  hhmm() { const m = Math.floor(this.time) % 1440; return String(Math.floor(m / 60)).padStart(2, '0') + ':' + String(m % 60).padStart(2, '0'); }
  // music slot, boundaries follow the clear-sky keyframes (dawn 6:00 -> noon -> warm 15:00 -> dusk 18:40 -> night 22:00)
  slot() { const t = this.time % 1440; return t >= 300 && t < 660 ? 'sunrise' : t >= 660 && t < 900 ? 'midday' : t >= 900 && t < 1200 ? 'sunset' : 'midnight'; }

  frame(file) { return this.data.frames[file]; }
  // colour set of a keyframe pair: sun light, ambient, fog, clouds
  colors(A, B, k) {
    const sunOf = f => { const p = f.planets.find(p => p.xml === 'planets/sun.xml'); return p ? (p.color || [1, 1, 1]).map(v => v * (p.intensity ?? 1)) : [1, 1, 1]; };
    return { sun: lerpArr(sunOf(A), sunOf(B), k), amb: lerpArr(A.ambient, B.ambient, k), fog: lerpArr(A.fogColor, B.fogColor, k), cloud: lerpArr(A.cloudColor, B.cloudColor, k),
      fs: lerp(A.fogStart ?? 0.4, B.fogStart ?? 0.4, k), fe: lerp(A.fogEnd ?? 0.95, B.fogEnd ?? 0.95, k), cover: lerp(A.cloudCover ?? 0.3, B.cloudCover ?? 0.3, k) };
  }
  // grade (per-channel ratios + fog range/cover targets) of the current region mix relative to the default atmosphere
  grade(base) {
    const keys = ['sun', 'amb', 'fog', 'cloud'], acc = {}, W = [...this.regionW.values()].reduce((a, b) => a + b, 0) || 1;
    for (const k of keys) acc[k] = [0, 0, 0];
    let fs = 0, fe = 0, cover = 0, wsum = 0;
    this._ref ||= this.colors(this.frame(REF_FRAME), this.frame(REF_FRAME), 0);
    for (const [name, w] of this.regionW) {
      const list = REGION_FRAMES[name]; if (!list || w < 0.001) continue;
      const cs = list.map(e => {
        if (e.config) { const [A, B, k] = this.blend(e.config); return { c: this.colors(A, B, k), ref: base }; }
        const f = this.frame(e); return f && { c: this.colors(f, f, 0), ref: this._ref };
      }).filter(Boolean);
      if (!cs.length) continue;
      for (const { c, ref } of cs) {
        const ww = w / cs.length; wsum += ww;
        for (const k of keys) for (let i = 0; i < 3; i++) acc[k][i] += ww * Math.log(clampR((c[k][i] + 0.02) / (ref[k][i] + 0.02)));
        fs += ww * c.fs; fe += ww * c.fe; cover += ww * c.cover;
      }
    }
    const g = { w: wsum / Math.max(W, 1) };
    for (const k of keys) g[k] = acc[k].map(v => Math.exp(v / Math.max(W, 1)));
    // blend range/cover toward the region by its total weight (the rest stays at the default atmosphere)
    g.fs = wsum ? fs / wsum : 0; g.fe = wsum ? fe / wsum : 0; g.cover = wsum ? cover / wsum : 0; g.mix = Math.min(1, wsum / Math.max(W, 1));
    return g;
  }
  blend(cfg = this.config) {
    const list = [...this.data.configs[cfg]].sort((a, b) => a.start - b.start);
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
    // smooth region weights (~4 s) so crossing a border fades the sky instead of snapping
    const ak = 1 - Math.exp(-dt / 4);
    for (const n of new Set([...this.regionW.keys(), ...this.regionT.keys()])) {
      const v = (this.regionW.get(n) || 0) + ((this.regionT.get(n) || 0) - (this.regionW.get(n) || 0)) * ak;
      if (v < 0.002 && !(this.regionT.get(n) > 0)) this.regionW.delete(n); else this.regionW.set(n, v);
    }
    const base = this.colors(A, B, k), gr = this.grade(base), GS = this.gradeStrength ?? 1;
    const gm = (c, ratio, p) => c && c.map((v, i) => v * Math.pow(ratio[i], p * GS));
    // planets, matched by their xml
    const planets = {};
    for (const pb of B.planets) {
      const pa = A.planets.find(p => p.xml === pb.xml) || pb;
      planets[pb.xml] = {
        dir: lerpArr(planetDir(pa.rot, pa.dist), planetDir(pb.rot, pb.dist), k),
        color: gm(lerpArr(pa.color, pb.color, k), pb.xml === 'planets/sun.xml' ? gr.sun : [1, 1, 1], 0.6), intensity: lerp(pa.intensity ?? 0, pb.intensity ?? 0, k), visible: pb.visible,
      };
    }
    s.planets = planets;
    const sun = planets['planets/sun.xml'];
    u.uSunDir.value.set(...sun.dir).normalize();
    u.uSunColor.value.copy(srgb(sun.color)).multiplyScalar(sun.intensity);
    const moons = ['planets/blue_moon.xml', 'planets/ghost_moon.xml', 'planets/red_moon.xml'];
    moons.forEach((m, n) => { const p = planets[m]; if (!p) return; u.uMoonDir.value[n].set(...p.dir).normalize(); u.uMoonColor.value[n].copy(srgb(p.color)).multiplyScalar(p.intensity * 0.5); });
    const amb = gm(lerpArr(A.ambient, B.ambient, k), gr.amb, 0.85);
    s.ambient = amb;
    u.uAmbSky.value.copy(srgb(amb)).multiplyScalar(1.5);
    u.uAmbGround.value.copy(srgb(amb)).multiplyScalar(0.7).multiply(new THREE.Color(0.8, 0.85, 0.7));
    const fog = gm(lerpArr(A.fogColor, B.fogColor, k), gr.fog, 0.9);
    s.fogRaw = fog;
    u.uFogColor.value.copy(srgb(fog));
    const rm = gr.mix * 0.5 * GS;   // fog range moves halfway toward the region's authored range
    const fs = lerp(lerp(A.fogStart, B.fogStart, k), gr.fs || A.fogStart, rm), fe = lerp(lerp(A.fogEnd, B.fogEnd, k), gr.fe || A.fogEnd, rm);
    u.uFogRange.value.set(fs * viewDist, fe * viewDist);
    u.uFogDensity.value = lerp(A.fogDensity ?? 1, B.fogDensity ?? 1, k);
    u.uViewDist.value = viewDist;
    s.cloudColor = gm(lerpArr(A.cloudColor, B.cloudColor, k), gr.cloud, 0.85);
    s.cloudCover = lerp(lerp(A.cloudCover ?? 0.3, B.cloudCover ?? 0.3, k), gr.cover || 0.3, gr.mix * 0.6 * GS);
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
