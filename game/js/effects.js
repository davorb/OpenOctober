// The game's combat effects (data/combatfx.json <- tools/build_combatfx.js, from common/specialeffect/specialeffects.xml): blood / impact splatters, arrows with
// their tracer, and the sounds the effects carry.  Only the effect classes combat needs are played:
//   BloodSplatter / Explosion OFFSET Particle System / Single Particle System   a billboard that plays a flipbook texture (particle_*_f16.dds, SFNumTextures x SFNumTextures
//                                       frames) over its life, scale and colour going start -> end (scales are metres: a body is ~1.7 m tall, the viewer's world is x1.5)
//   Random Single (Chance) 3D Audio     one of the listed wavs, attenuated between SFMinDistance and SFMaxDistance
//   Mesh / Mesh Simple + Tracer         the arrow: waw_arrow_01 with a ribbon of 40 trace points (colour SFTracerColorStart)
// Not played: debris / fountain / light classes (they need the game's particle shaders), the swing trail of se_weaponswing_* (audio only in the data).
import * as THREE from 'three';
import { DDSLoader } from 'three/addons/loaders/DDSLoader.js';

const WORLD = 1.5, GRAVITY = 9.8;
const PARTICLES = new Set(['BloodSplatter', 'Explosion OFFSET Particle System', 'Single Particle System', 'Explosion Particle System']);
const lerp = (a, b, t) => a + (b - a) * t;

export class CombatFx {
  constructor({ scene, world, sfx, factory, data }) {
    Object.assign(this, { scene, world, sfx, factory, data }); this.dds = new DDSLoader(); this.tex = new Map(); this.live = []; this.arrows = []; this.stuck = [];
  }
  texture(name) {
    if (!this.tex.has(name)) this.tex.set(name, new Promise(res => this.dds.load('/dds/' + name, t => { t.colorSpace = THREE.SRGBColorSpace; t.needsUpdate = true; res(t); }, undefined, () => res(null))));
    return this.tex.get(name);
  }
  /** Play the effect `name` (an se_* of combatfx.json) at world position `at` (feet of whoever was hit). */
  play(name, at, { vol = 1 } = {}) {
    const e = this.data[name]; if (!e) return;
    const one = (x, main) => {
      if (x.wavs) this.sfx.play(this.sfx.pick(x.wavs), { at, min: x.p.SFMinDistance ?? 5, max: x.p.SFMaxDistance ?? 50, vol });
      if (PARTICLES.has(x.cls)) this.particle(x, at, main);
      for (const k of x.kids || []) one(k, false);
    };
    one(e, true);
  }
  async particle(x, at, main) {
    const p = x.p, off = p.SFTargetPositionOffset || p.SFTargetOffset || [0, 1, 0];
    this.sprite(p.SFTextureName || '', Math.max(1, p.SFNumTextures | 0), new THREE.Vector3(at.x, at.y + off[1] * WORLD, at.z), {
      life: (main ? p.SFLifeTime : p.SFParticleLifeTime) || p.SFDuration || 1, delay: p.SFEffectDelayTime || 0, s0: p.SFStartScale ?? 1, s1: p.SFEndScale ?? 1,
      c0: p.SFStartColor || [1, 1, 1, 1], c1: p.SFEndColor || p.SFStartColor || [1, 1, 1, 1], gravity: p.SFGravityInfluence || 0 });
  }
  /** One billboard playing an n x n flipbook texture: size s0 -> s1 metres, colour c0 -> c1 (rgba) over `life` seconds. */
  async sprite(texName, n, pos, { life = 1, delay = 0, s0 = 1, s1 = 1, c0 = [1, 1, 1, 1], c1 = c0, gravity = 0, additive } = {}) {
    const tex = await this.texture(texName); if (!tex) return;
    const t = tex.clone(); t.needsUpdate = true; t.repeat.set(1 / n, 1 / n);
    additive ??= /fire|spark|glow|magic|lightning|flash/.test(texName);
    const m = new THREE.SpriteMaterial({ map: t, transparent: true, depthWrite: false, blending: additive ? THREE.AdditiveBlending : THREE.NormalBlending, fog: false });
    const s = new THREE.Sprite(m); s.position.copy(pos); s.renderOrder = 22; this.scene.add(s);
    this.live.push({ s, m, t, n, age: -delay, life, p: { SFStartScale: s0, SFEndScale: s1, SFStartColor: c0, SFEndColor: c1, SFGravityInfluence: gravity }, vy: 0 });
  }
  /** A glowing bolt (staff / spell missile): fire-flipbook head with a short trail of fading puffs. onEnd(pos, hitGround) like arrow(). */
  bolt({ from, dir, speed = 45, range = 120, tex = 'particle_fire_04_f16.dds', n = 4, color = [1, 0.55, 0.15, 1], onEnd }) {
    const a = { bolt: true, pos: from.clone(), vel: dir.clone().normalize().multiplyScalar(speed), left: range, onEnd, tex, n, color, puff: 0, hist: [], N: 0 };
    this.sprite(tex, n, from, { life: range / speed, s0: 1.1, s1: 1.1, c0: color, additive: true }).then(() => { const f = this.live[this.live.length - 1]; a.head = f; });
    this.arrows.push(a); return a;
  }

  /** An arrow: mesh + tracer from `from` along `dir` at `speed` m/s for at most `range` m; `onEnd(pos, hitGround)` when it lands.  Returns the arrow entity. */
  async arrow({ from, dir, speed = 100, range = 300, onEnd }) {
    const src = await this.factory.static('waw_arrow_01'), g = new THREE.Group();
    if (src) { const m = src.clone(true); m.scale.setScalar(WORLD); m.traverse(o => { if (o.isMesh) o.frustumCulled = false; }); g.add(m); }
    g.position.copy(from); g.lookAt(from.clone().add(dir)); this.scene.add(g);
    const N = 24, geo = new THREE.BufferGeometry().setAttribute('position', new THREE.BufferAttribute(new Float32Array(N * 3), 3)), trail = new THREE.Line(geo, new THREE.LineBasicMaterial({ color: 0xccccff, transparent: true, opacity: 0.55, depthWrite: false, fog: false }));
    trail.frustumCulled = false; this.scene.add(trail);
    const a = { g, trail, pos: from.clone(), vel: dir.clone().normalize().multiplyScalar(speed), left: range, hist: [], N, onEnd, done: false };
    this.arrows.push(a); return a;
  }

  update(dt) {
    for (let i = this.live.length - 1; i >= 0; i--) {
      const f = this.live[i]; f.age += dt; const k = f.age / f.life;
      if (k >= 1) { this.scene.remove(f.s); f.m.dispose(); f.t.dispose(); this.live.splice(i, 1); continue; }
      if (k < 0) { f.s.visible = false; continue; } f.s.visible = true;
      const p = f.p, c0 = p.SFStartColor || [1, 1, 1, 1], c1 = p.SFEndColor || c0, sc = lerp(p.SFStartScale ?? 1, p.SFEndScale ?? 1, k) * WORLD, fr = Math.min(f.n * f.n - 1, (k * f.n * f.n) | 0);
      f.m.color.setRGB(lerp(c0[0], c1[0], k) * 1.6 + 0.15, lerp(c0[1], c1[1], k) * 1.6, lerp(c0[2], c1[2], k) * 1.6); f.m.opacity = lerp(c0[3], c1[3], k); f.s.scale.set(sc, sc, 1);
      f.t.offset.set((fr % f.n) / f.n, ((f.n - 1 - ((fr / f.n) | 0)) / f.n));
      f.vy -= (p.SFGravityInfluence || 0) * GRAVITY * dt; f.s.position.y += f.vy * dt;
    }
    for (let i = this.arrows.length - 1; i >= 0; i--) {
      const a = this.arrows[i];
      if (a.bolt) { a.pos.addScaledVector(a.vel, dt); a.left -= a.vel.length() * dt; if (a.head) { a.head.s.position.copy(a.pos); a.head.age = Math.min(a.head.age, a.head.life * 0.5); }
        if ((a.puff -= dt) <= 0) { a.puff = 0.035; this.sprite(a.tex, a.n, a.pos.clone(), { life: 0.45, s0: 0.8, s1: 0.2, c0: a.color, c1: [a.color[0], a.color[1] * 0.5, 0, 0], additive: true }); }
        const gb = this.world.groundHeight(a.pos.x, a.pos.z, a.pos.y + 2), hitb = gb != null && gb > -1e8 && a.pos.y <= gb;
        if (hitb || a.left <= 0 || a.done) { this.arrows.splice(i, 1); if (a.head) a.head.age = a.head.life; if (hitb && !a.done) this.sprite('particle_fire_04_f16.dds', 4, a.pos.clone(), { life: 0.6, s0: 1.5, s1: 3, c0: a.color, c1: [1, 0.3, 0, 0], additive: true }); a.onEnd?.(a.pos, hitb); a.dead = true; }
        continue; }
      a.vel.y -= GRAVITY * 0.35 * dt; const step = a.vel.length() * dt, prev = a.pos.clone();
      a.pos.addScaledVector(a.vel, dt); a.left -= step;
      const gh = this.world.groundHeight(a.pos.x, a.pos.z, a.pos.y + 2), hit = gh != null && gh > -1e8 && a.pos.y <= gh;
      a.g.position.copy(a.pos); a.g.lookAt(a.pos.clone().add(a.vel));
      a.hist.unshift(a.pos.x, a.pos.y, a.pos.z); if (a.hist.length > a.N * 3) a.hist.length = a.N * 3;
      const arr = a.trail.geometry.attributes.position.array; for (let j = 0; j < a.N; j++) { const s = Math.min(j, a.hist.length / 3 - 1) * 3; arr[j * 3] = a.hist[s]; arr[j * 3 + 1] = a.hist[s + 1]; arr[j * 3 + 2] = a.hist[s + 2]; }
      a.trail.geometry.attributes.position.needsUpdate = true;
      if (hit || a.left <= 0 || a.done) {
        this.arrows.splice(i, 1); this.scene.remove(a.trail); a.trail.geometry.dispose();
        if (hit && !a.done) { a.pos.y = gh; a.g.position.y = gh + 0.05; this.stuck.push({ g: a.g, age: 0 }); } else this.scene.remove(a.g);
        a.onEnd?.(a.pos, hit); a.dead = true;
      }
    }
    for (let i = this.stuck.length - 1; i >= 0; i--) { const s = this.stuck[i]; s.age += dt; if (s.age > 20) { this.scene.remove(s.g); this.stuck.splice(i, 1); } }   // se_arrow_impact lasts 20 s
  }
}
