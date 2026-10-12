// Plays the game's own player clips (data/playeranims/ <- tools/build_playeranims.js) on an avatar from js/avatars.js.
// Lower layer: a *_primary clip chosen from the stance (nothing drawn / one-handed / two-handed / halberd) and the ground speed, crossfaded.
// Upper layer: a *_secondary clip (attack, parry, stab, bow, magic, (un)sheathe ...) that only names the 31 upper-body bones, applied over the primary pose with a
// short fade in and out so the legs keep walking.  Each avatar part (legs/torso/arms/head) has its own copy of the skeleton, so every clip is bound to every part.
import { loadPlayerClips, bindClips, applyClip } from './mobanim.js';

export const FILE = { hum: 'hum', alf: 'alfar', dwf: 'dwarf', ork: 'ork', mir: 'mirdain', mhm: 'mahirim' };   // race id -> animation file prefix
const PREFIX = { none: 'normal', one: 'attackmode', two: 'attackmode_2handed', halberd: 'attackmode_halberd' };
// ground speed (m/s in the viewer: the game's 2.5 / 3.7 / 5.5 times PLAYER_SCALE) at which each locomotion clip plays at its authored pace
const GAIT = [['idle', 0], ['walk_forward', 3.75], ['run_forward', 5.55], ['sprint_forward', 8.25]];
const FADE = 0.2, UP_FADE = 0.08;
/** classes whose drawn pose is the end of an upper-body clip rather than a stance of the primary clips */
const HOLD = { bow: 'bow_aim', staff: 'unsheathe_staff' };

/** The stance clips to use for a weapon group (data/weapons.json `group`), and the secondary-clip family its attacks come from. */
export const STANCE = {
  sword: ['one', 'singlehand'], axe: ['one', 'singlehand'], club: ['one', 'singlehand'], sithra: ['one', 'singlehand'], knife: ['one', '2daggers'],
  greatsword: ['two', '2handed'], greataxe: ['two', '2handed'], greatclub: ['two', '2handed'], polearm: ['halberd', 'halberd'],
  bow: ['one', 'bow'], staff: ['none', 'magic'], thrown: ['one', 'attack_throw'], shield: ['one', 'singlehand'],
};

export class PlayerAnim {
  static async create(av) {
    const clips = await loadPlayerClips(`${FILE[av.race] || av.race}_${av.g}`); if (!clips) return null;
    return new PlayerAnim(av, clips);
  }
  constructor(av, clips) {
    this.av = av; this.clips = clips; this.parts = av.rigs.map(r => bindClips(r, clips));
    this.stance = 'none'; this.cur = null; this.prev = null; this.t = 0; this.pt = 0; this.fade = 1; this.up = null;
  }
  setWeapon(group) { this.holdT = 0; this.stance = group && !this.sheathed ? STANCE[group]?.[0] || 'one' : 'none'; this.group = group; }
  /** drawn weapon or not (a drawn one switches to the attack-mode clips) */
  setDrawn(v) { this.sheathed = !v; this.setWeapon(this.group); }

  /** The primary clip for this stance / speed, falling back to the one-handed set when the stance lacks it (the halberd has no backward walk). */
  pick(speed, air, swim, move) {
    if (this.dead && this.clips.gank_death) return 'gank_death';
    if (swim) return speed > 0.3 ? (speed > 4 ? 'swim_sprint' : 'swim_forward') : 'swim_idle';
    if (air) return 'falling';
    let g = GAIT[0][0]; for (const [n, v] of GAIT) if (speed >= Math.max(0.15, v * 0.6)) g = n;
    // moving sideways / backwards relative to where the body faces (our own avatar; other players' only report speed)
    if (move && g !== 'idle') { const side = Math.abs(move.x) > Math.abs(move.z) * 0.8; const dir = side ? (move.x > 0 ? 'run_right' : 'run_left') : move.z < -0.3 ? 'walk_backward' : g;
      for (const p of [PREFIX[this.stance], PREFIX.one, PREFIX.none]) if (this.clips[`${p}_${dir}`]) return `${p}_${dir}`; }
    for (const p of [PREFIX[this.stance], PREFIX.one, PREFIX.none]) if (this.clips[`${p}_${g}`]) return `${p}_${g}`;
    return null;
  }
  /** One upper-body clip (a name from data/playeranims, e.g. 'singlehand_attack_01'); returns its duration in seconds, 0 if the file lacks it. */
  play(name, rate = 1) {
    const c = this.clips[name]; if (!c) return 0;
    this.up = { name, t: 0, rate, dur: c.period / rate }; return this.up.dur;
  }
  get busy() { return !!this.up; }

  frame(name, t) { const c = this.clips[name]; return name === 'gank_death' ? Math.min(t, c.period) / c.dt : (t % c.period) / c.dt; }   // dying plays once and stays down
  update(dt, speed = 0, air = false, swim = false, move = null) {
    const want = this.pick(speed, air, swim, move);
    if (want && want !== this.cur) {
      const loco = n => /_(walk|run|sprint)_forward$/.test(n || '');
      if (this.cur) { this.prev = this.cur; this.pt = this.t; this.fade = 0; }
      this.t = loco(want) && loco(this.cur) ? this.t * this.clips[want].period / this.clips[this.cur].period : 0; this.cur = want;
    }
    if (!this.cur) return;
    const gait = GAIT.find(([n]) => this.cur.endsWith(n)) || (/run_(left|right)$/.test(this.cur) ? GAIT[2] : /walk_backward$/.test(this.cur) ? GAIT[1] : null), ref = gait && gait[1] ? gait[1] : 0, rate = ref && speed > 0.15 ? Math.min(1.5, Math.max(0.5, speed / ref)) : 1;
    this.t += dt * rate; this.pt += dt; if (this.fade < 1) this.fade = Math.min(1, this.fade + dt / FADE);
    if (this.dead) this.up = null;
    this.holdT = (this.holdT || 0) + dt;
    const u = this.up; if (u) { u.t += dt; if (u.t >= u.dur) this.up = null; }
    for (const p of this.parts) {
      if (this.prev && this.fade < 1) { applyClip(p, this.prev, this.frame(this.prev, this.pt), 1); applyClip(p, this.cur, this.frame(this.cur, this.t), this.fade); }
      else applyClip(p, this.cur, this.frame(this.cur, this.t), 1);
      const hc = HOLD[this.group]; if (!this.up && !this.dead && hc && !this.sheathed && this.clips[hc]) { const c = this.clips[hc]; applyClip(p, hc, Math.min(c.nk - 1, this.holdT * (this.group === 'bow' ? 2 : 1) / c.dt), Math.min(1, this.holdT * 4)); }   // a drawn bow stays at the aim pose, a drawn staff at the end of its unsheathe clip (held upright)
      if (u && this.up) { const w = Math.min(1, u.t / UP_FADE, (u.dur - u.t) / UP_FADE), c = this.clips[u.name]; applyClip(p, u.name, Math.min(c.nk - 1, u.t * u.rate / c.dt), Math.max(0, w)); }
    }
  }
}
