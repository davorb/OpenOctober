// Music director: plays the tracks the game's env_music_settings.xml assigns to the area (tile list) the camera is over,
// for the current time of day (Sunrise / Midday / Sunset / Midnight), with crossfades. Plus the environmental ambience loops
// (forest, river, mountain wind + random gusts/wolves) described by env_audio_list.xml.
const pick = (list, not) => { const l = list.filter(x => x !== not); const a = l.length ? l : list; return a[Math.floor(Math.random() * a.length)]; };
const pretty = f => f.replace(/\.ogg$/i, '').replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
const SLOTS = ['sunrise', 'midday', 'sunset', 'midnight'];

export class Music {
  constructor(areas, env, onTrack) {
    this.env = env; this.onTrack = onTrack || (() => {});
    this.byTile = new Map(); for (const a of areas) for (const t of a.tiles) if (!this.byTile.has(t)) this.byTile.set(t, a);
    this.area = null; this.pending = null; this.pendingT = 0; this.slot = null;
    this.vol = 0.55; this.muted = false; this.enabled = false;
    this.cur = null; this.old = [];            // {el, target, fade}
    this.lastTrack = null;
  }
  enable() { this.enabled = true; }
  toggle() { this.muted = !this.muted; return !this.muted; }
  areaFor(tile) { return this.byTile.get(tile); }
  list(area, slot) {
    let l = area[slot];
    if (l?.length) return l;
    for (const s of SLOTS) if (area[s]?.length) return area[s];   // slot without music: fall back to any slot of the area
    return [];
  }
  setTile(tileName, dt) {
    const a = tileName ? this.areaFor(tileName) : null;
    if (!a || a === this.area) { this.pending = null; return; }
    if (this.pending !== a) { this.pending = a; this.pendingT = 0; }
    this.pendingT += dt;
    if (this.pendingT > 1.5 || !this.area) { this.area = a; this.pending = null; this._play(this.list(a, this.env.slot()), 6); }
  }
  _play(list, fadeIn) {
    if (!this.enabled || !list.length) return;
    const file = pick(list, this.lastTrack); this.lastTrack = file;
    const el = new Audio('/audio/ambience/' + encodeURIComponent(file));
    el.volume = 0; el.preload = 'auto';
    el.addEventListener('ended', () => { if (this.cur?.el === el) { this.old.push(this.cur); this.cur = null; this._play(this.list(this.area, this.env.slot()), 3); } });
    el.addEventListener('error', () => { console.warn('music failed', file); if (this.cur?.el === el) { this.cur = null; setTimeout(() => this._play(this.list(this.area, this.env.slot()), 3), 1500); } });
    if (this.cur) this.old.push(this.cur);
    this.cur = { el, fade: 0, fadeIn, file };
    el.play().catch(() => {});
    this.onTrack(pretty(file), this.area?.name);
  }
  update(dt) {
    const slot = this.env.slot();
    if (slot !== this.slot) { this.slot = slot; this.slotChanged = true; }
    // when the day part changes, move to that part's playlist at the next gap in the music (or now if the playing track isn't in it)
    if (this.slotChanged && this.cur && this.area) {
      const l = this.list(this.area, slot);
      if (l.length && !l.includes(this.cur.file) && this.cur.el.currentTime > 30) { this.slotChanged = false; this._play(l, 8); }
    }
    const master = this.muted ? 0 : this.vol;
    if (this.cur) { this.cur.fade = Math.min(1, this.cur.fade + dt / this.cur.fadeIn); this.cur.el.volume = Math.min(1, master * this.cur.fade * this.cur.fade * (3 - 2 * this.cur.fade)); }
    for (const o of this.old) { o.fade = Math.max(0, o.fade - dt / 6); o.el.volume = Math.max(0, Math.min(1, master * o.fade)); if (o.fade <= 0) { o.el.pause(); o.el.src = ''; o.dead = true; } }
    this.old = this.old.filter(o => !o.dead);
  }
}

// Environmental bed. Volumes follow env_audio_list.xml height curves (Volume vs. metres above ground).
const curve = (pts, h) => { if (h <= pts[0][0]) return pts[0][1]; for (let i = 1; i < pts.length; i++) if (h <= pts[i][0]) { const k = (h - pts[i - 1][0]) / (pts[i][0] - pts[i - 1][0]); return pts[i - 1][1] + k * (pts[i][1] - pts[i - 1][1]); } return pts[pts.length - 1][1]; };
const FOREST_H = [[0, 0.7], [5, 1], [30, 1], [80, 0]];
export class Ambience {
  constructor(env) {
    this.env = env; this.enabled = false; this.master = 0.5;
    const loop = f => { const a = new Audio('/audio/environmental/' + f); a.loop = true; a.volume = 0; return a; };
    this.forest = loop('panozk_test_forest_stereo_loop.ogg'); this.river = loop('panozk_test_river_stereo_loop.ogg');
    this.wind = loop('panozk_test_mountain_top_wind_stereo_loop.ogg'); this.birds = loop('amb_birds_forestpark.ogg');
    this.gustT = 8; this.wolfT = 30; this.vols = { forest: 0, river: 0, wind: 0, birds: 0 };
  }
  enable() { this.enabled = true; for (const a of [this.forest, this.river, this.wind, this.birds]) a.play().catch(() => {}); }
  oneShot(files, vol) { const a = new Audio('/audio/environmental/' + pick(files)); a.volume = Math.min(1, vol); a.play().catch(() => {}); }
  // ctx: { y, agl, treeDensity (0..1), riverDist (m), muted }
  update(dt, ctx) {
    if (!this.enabled) return;
    const m = ctx.muted ? 0 : this.master;
    const day = this.env.state.daylight ?? 1;
    const target = {
      forest: ctx.treeDensity * curve(FOREST_H, ctx.agl),
      river: (1 - Math.min(1, Math.max(0, (ctx.riverDist - 15) / 220))) * curve([[0, 0.8], [10, 1], [60, 0.5], [140, 0]], ctx.agl),
      wind: Math.min(1, Math.max(0, (ctx.y - 300) / 260)) * 0.9 + Math.min(1, ctx.agl / 400) * 0.5,
      birds: ctx.treeDensity * curve(FOREST_H, ctx.agl) * day * 0.7,
    };
    for (const k of Object.keys(target)) {
      this.vols[k] += (Math.min(1, target[k]) - this.vols[k]) * Math.min(1, dt * 0.6);
      this[k].volume = Math.max(0, Math.min(1, this.vols[k] * m));
    }
    // randomizers (Mountain Sound): wind gusts every 10±5 s, wolves every 22±6 s, only up high / at night respectively
    this.gustT -= dt; this.wolfT -= dt;
    if (this.gustT <= 0) { this.gustT = 10 + (Math.random() - 0.5) * 10; if (this.vols.wind > 0.25) this.oneShot([1, 2, 3, 4, 5, 6].map(i => `rand_wind_gust_${i}.wav`), 0.3 * m); }
    if (this.wolfT <= 0) { this.wolfT = 22 + (Math.random() - 0.5) * 12; if (this.vols.wind > 0.3 && day < 0.6) this.oneShot([1, 2, 3, 4, 5, 6, 7, 8].map(i => `rand_wolf_${i}.wav`), 0.3 * m); }
  }
}
