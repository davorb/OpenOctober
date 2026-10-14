// One-shot sound effects over Web Audio (the music / ambience in music.js use plain Audio elements, which can't pan or be pooled for rapid swings).
// Files are the game's wavs under export/audio, served at /audio/; a sound can sit at a world position and is then attenuated and panned from the camera like the
// game's 3D audio effects (SFMinDistance / SFMaxDistance of the effect, e.g. 2 / 30 m for a sword swing, 5 / 50 m for a blow landing).
import * as THREE from 'three';

const fwd = new THREE.Vector3(), up = new THREE.Vector3(0, 1, 0);

export class Sfx {
  constructor() { this.ctx = null; this.master = null; this.buf = new Map(); this.muted = false; this.volume = 0.8; }
  /** call from a user gesture (the intro click): browsers keep the context suspended until then */
  enable() {
    if (!this.ctx) { try { this.ctx = new (window.AudioContext || window.webkitAudioContext)(); this.master = this.ctx.createGain(); this.master.connect(this.ctx.destination); this.master.gain.value = this.muted ? 0 : this.volume; } catch { return; } }
    this.ctx.resume?.();
  }
  setMuted(m) { this.muted = m; if (this.master) this.master.gain.value = m ? 0 : this.volume; }
  /** AudioBuffer of a wav path as found in the data ('/weapons/general/df_weapon_swing_sword_01.wav'); null if it can't be had. */
  load(path) {
    if (!this.buf.has(path)) this.buf.set(path, (async () => {
      if (!this.ctx) return null;
      try { const r = await fetch('/audio' + encodeURI(path).replace(/&/g, '%26')); if (!r.ok) return null; return await this.ctx.decodeAudioData(await r.arrayBuffer()); } catch { return null; }
    })());
    return this.buf.get(path);
  }
  /** Start loading sounds before they are needed (rapid first use would otherwise miss the swing). */
  preload(paths) { if (this.ctx) for (const p of paths) this.load(p); }
  pick(list) { return list && list.length ? list[(Math.random() * list.length) | 0] : null; }

  /** Play a sound: opts { vol, rate, at: THREE.Vector3 (world position), min, max (metres, for `at`) }. Fire and forget. */
  async play(path, { vol = 1, rate = 1, at = null, min = 2, max = 40 } = {}) {
    if (!path || !this.ctx || this.muted) return;
    const b = await this.load(path); if (!b || !this.ctx) return;
    const src = this.ctx.createBufferSource(); src.buffer = b; src.playbackRate.value = rate;
    const g = this.ctx.createGain(); g.gain.value = vol; src.connect(g);
    if (at) {
      const p = this.ctx.createPanner(); p.panningModel = 'equalpower'; p.distanceModel = 'inverse'; p.refDistance = min; p.maxDistance = max; p.rolloffFactor = 1.4;
      p.positionX.value = at.x; p.positionY.value = at.y; p.positionZ.value = at.z; g.connect(p); p.connect(this.master);
      // inverse distance never reaches zero: fade out the last quarter before max distance so far swings don't linger
      const l = this.ctx.listener, d = Math.hypot(at.x - l.positionX.value, at.y - l.positionY.value, at.z - l.positionZ.value);
      if (d > max) return; if (d > max * 0.75) g.gain.value = vol * (1 - (d - max * 0.75) / (max * 0.25));
    } else g.connect(this.master);
    src.start();
  }
  /** Loop a sound until `stop()` is called on the handle it returns; opts as for `play`. Without a distance cut-off: the caller only starts it for things it can place. */
  loop(path, { vol = 1, at = null, min = 2 } = {}) {
    const h = { src: null, stopped: false, stop() { this.stopped = true; try { this.src?.stop(); } catch {} this.src = null; } };
    (async () => {
      const b = path && this.ctx ? await this.load(path) : null; if (!b || h.stopped || !this.ctx) return;
      const src = this.ctx.createBufferSource(); src.buffer = b; src.loop = true;
      const g = this.ctx.createGain(); g.gain.value = vol; src.connect(g);
      if (at) {
        const p = this.ctx.createPanner(); p.panningModel = 'equalpower'; p.distanceModel = 'inverse'; p.refDistance = min; p.rolloffFactor = 1.4;
        p.positionX.value = at.x; p.positionY.value = at.y; p.positionZ.value = at.z; g.connect(p); p.connect(this.master);
      } else g.connect(this.master);
      src.start(); h.src = src;
    })();
    return h;
  }
  /** keep the listener on the camera (call every frame) */
  listen(camera) {
    if (!this.ctx) return; const l = this.ctx.listener; camera.getWorldDirection(fwd);
    if (l.positionX) { l.positionX.value = camera.position.x; l.positionY.value = camera.position.y; l.positionZ.value = camera.position.z; l.forwardX.value = fwd.x; l.forwardY.value = fwd.y; l.forwardZ.value = fwd.z; l.upX.value = up.x; l.upY.value = up.y; l.upZ.value = up.z; }
  }
}
