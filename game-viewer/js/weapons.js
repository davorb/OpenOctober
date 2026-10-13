// The player's weapon: what is wielded (data/weapons.json <- tools/build_weapons.js), the first-person view (js/viewmodel.js), attacks with their game sounds
// (js/sfx.js, se_weaponswing_*: the whoosh plays 0.18 s into the swing like the skills' SFOffsetStart) and effects (js/effects.js), and the armory panel.
//
//   Left mouse   attack (melee: slash / backhand / chop / stab in a combo; bow: hold to draw, release to shoot; staff: cast; thrown: throw)
//   Right mouse  hold to guard (melee)      R  draw / sheathe      1-9, 0  hotbar (one weapon per class)      I  armory: every weapon in the game data
//
// Durations are the lengths of the game's own animation clips (weapons.json `clipDur`).  Combat itself is the server's (game-server/src/combat.rs): this class sends what
// the player does through `net` (equip / attack / block / shoot) and plays what comes back (`hit`, `died`, ... see onHit).  Without a server the swings still play.
import * as THREE from 'three';
import { ViewModel } from './viewmodel.js';
import { STANCE } from './playeranim.js';

const HOTBAR = ['sword', 'axe', 'club', 'greatsword', 'greataxe', 'greatclub', 'polearm', 'knife', 'bow', 'staff'];
const COMBO = { sword: ['slash', 'backhand', 'chop'], axe: ['chop', 'slash', 'backhand'], club: ['chop', 'slash', 'backhand'], knife: ['stab', 'slash', 'backhand'], sithra: ['slash', 'backhand', 'stab'],
  greatsword: ['slash', 'chop', 'backhand'], greataxe: ['chop', 'slash'], greatclub: ['chop', 'slash', 'backhand'], polearm: ['stab', 'chop', 'slash'], shield: ['slash'] };
const IMPACT = { sword: 'slashing', axe: 'slashing', greatsword: 'slashing', greataxe: 'slashing', polearm: 'slashing', sithra: 'slashing', club: 'bludgeoning', greatclub: 'bludgeoning', knife: 'piercing' };
const PARRY = { knife: 'dagger', polearm: 'halberd', greatsword: 'two_handed', greataxe: 'two_handed', greatclub: 'two_handed', shield: 'shield' };
const SWING_AT = 0.18;                                          // SFOffsetStart of the skills' se_weaponswing_* effect
const ARCHERY = '/weapons/archery/df_weapon_archery_';
const GROUP_NAMES = { sword: 'Swords', axe: 'Axes', club: 'Clubs & maces', greatsword: 'Greatswords', greataxe: 'Greataxes', greatclub: 'Greatclubs', polearm: 'Polearms', knife: 'Daggers', sithra: 'Sithras', bow: 'Bows', staff: 'Staves', shield: 'Shields', thrown: 'Thrown' };

/** The upper-body clip (data/playeranims) of one attack: family 'singlehand' | '2handed' | 'halberd' | '2daggers', shape 'slash' ... */
export function attackClip(group, kind) {
  const [, fam] = STANCE[group] || ['one', 'singlehand'];
  if (group === 'bow') return 'bow_fire'; if (group === 'staff') return 'magic_fire_01'; if (group === 'thrown') return 'attack_throw';
  return { slash: `${fam}_attack_01`, backhand: `${fam}_attack_02`, chop: `${fam}_vertical_attack_01`, stab: `${fam}_stab` }[kind] || `${fam}_attack_01`;
}

/** the guard clip of a weapon class */
export const parryClip = group => `${STANCE[group]?.[1] || 'singlehand'}_parrying`;

export class Weapons {
  constructor({ camera, post, world, factory, sfx, fx, net, toast, $, profile, light }) {
    Object.assign(this, { camera, post, world, factory, sfx, fx, net, toast, $, profile, lightSrc: light });
    this.data = null; this.id = null; this.drawn = false; this.combo = 0; this.vm = new ViewModel(factory); this.vm.onhit = () => this.landed();
    this.post.overlay = this.vm; this.down = { 0: false, 2: false }; this.draw = null; this.hp = { hp: 100, max: 100 }; this.dead = false; this.hotbar = []; this.owned = null; this.canFight = true; this.self = null; this.eye = null;
  }
  async init() {
    this.data = await fetch('/data/weapons.json').then(r => r.json());
    const W = this.data.weapons;
    this.rebuildHotbar();
    this.buildUI();
  }
  /** One weapon per class on the hotbar: the plainest in the game, or - when the server only lets us wield what we own (`owned`) - the best we carry or hold. */
  rebuildHotbar() {
    const W = this.data.weapons, mine = id => !this.owned || this.owned.has(id) || id === this.id;
    this.hotbar = HOTBAR.map(g => {
      const list = Object.entries(W).filter(([id, w]) => w.group === g && w.mesh && w.src === 'tree' && !w.natural && w.rank && mine(id)).sort((a, b) => a[1].rank - b[1].rank || a[1].name.localeCompare(b[1].name));
      return (this.owned ? list[list.length - 1] : list[0])?.[0];
    }).filter(Boolean);
  }
  /** The server says what is in our pack (`own`: it enforces ownership): the hotbar and the armory follow. */
  setOwned(items, own) {
    this.owned = own ? new Set(Object.keys(items)) : null;
    if (this.data) { this.rebuildHotbar(); this.refreshUI(); if (this.$('armory')?.classList.contains('on')) this.drawArmory(); }
  }
  get w() { return this.data?.weapons[this.id] || null; }
  /** drawn weapons (except magic staves) are used in third person; sheathed or a staff = first person */
  get third() { const w = this.w; return !!(w && this.drawn && this.canFight && w.group !== 'staff' && w.group !== 'thrown' && !this.dead); }
  /** where shots start and the aim ray begins: the body's head in third person, else the camera */
  origin() { return this.third && this.eye ? this.eye : this.camera.position; }
  dur(name, d = 1) { return this.data?.clipDur[name] ?? d; }

  // ---- equip / draw
  /** `silent`: the server told us (the weapon it gave us, or the one it says we hold): do not tell it back. An empty id puts the weapon away for good. */
  async equip(id, draw = true, silent = false) {
    if (!id) { this.id = null; this.drawn = false; this.cancelDraw(); await this.vm.setWeapon(null); this.vm.visible = false; this.self?.wield(null, false); this.refreshUI(); if (!silent) this.net?.equip('', false); return; }
    const w = this.data.weapons[id]; if (!w) return;
    this.id = id; this.drawn = draw; this.combo = 0; this.cancelDraw(); await this.vm.setWeapon(draw ? w : null); this.vm.visible = draw;
    if (!silent) this.net?.equip(id, draw);
    this.self?.wield(w, draw); this.refreshUI(); this.sfx.play(this.sfx.pick(['/weapons/general/df_weapon_swing_01.wav']), { vol: 0.35, rate: 1.3 });
    this.toast(`${w.name}${w.rank ? ' (rank ' + w.rank + ')' : ''}`);
  }
  async toggleDrawn() { if (!this.id) return; this.drawn = !this.drawn; this.cancelDraw(); await this.vm.setWeapon(this.drawn ? this.w : null); this.vm.visible = this.drawn; this.net?.equip(this.id, this.drawn); this.self?.wield(this.w, this.drawn); this.refreshUI(); }

  // ---- input
  mouse(button, isDown) {
    if (this.dead) return; this.down[button] = isDown; const w = this.w;
    if (isDown && !this.canFight) { if (button === 0) this.toast('Walk (G) to fight'); return; }
    if (button === 0 && isDown) { if (!w) return; if (!this.drawn) { this.toggleDrawn(); return; } this.press(); }
    if (button === 0 && !isDown && this.draw) this.releaseBow();
    if (button === 2 && w && this.drawn && !['bow', 'staff', 'thrown'].includes(w.group)) { this.vm.setHold(isDown ? 'block' : null); this.net?.block(isDown); if (isDown) this.self?.play(parryClip(w.group)); if (isDown) this.sfx.play('/weapons/parry/df_weapon_parry_one_handed_1.wav', { vol: 0.15, rate: 1.6 }); }
  }
  press() {
    const w = this.w, g = w.group; if (this.vm.busy) return;
    if (g === 'bow') return this.startDraw();
    if (g === 'staff') return this.attack('cast', 'magic_fire_01', 0.8);
    if (g === 'thrown') return this.attack('throw', 'attack_throw', 0.8);
    const list = COMBO[g] || COMBO.sword, kind = list[this.combo++ % list.length], clip = attackClip(g, kind); this.attack(kind, clip, this.dur(clip, 1));
  }
  attack(kind, clip, dur) {
    const w = this.w, g = w.group; if (!this.vm.swing(kind, dur)) return;
    const grp = this.data.groups[g], at = this.camera.position;
    if (grp?.sounds) setTimeout(() => this.sfx.play(this.sfx.pick(grp.sounds), { vol: 0.9, at: null, rate: 0.95 + Math.random() * 0.1 }), SWING_AT * 1000);
    this.net?.attack({ kind, clip, dur, group: g }); this.self?.play(clip);
    if (g === 'staff') setTimeout(() => this.cast(), dur * 0.45 * 1000);
    if (g === 'thrown') setTimeout(() => this.cast(), dur * 0.45 * 1000);
  }
  /** the blow lands (45% into the swing): the server decides who it hits, the whoosh and the animation are ours */
  landed() {}

  // ---- bow: hold to draw, release to shoot
  startDraw() {
    if (this.draw) return; this.draw = { t: 0 }; this.vm.setHold('draw'); this.net?.bow('load'); this.self?.play('bow_load');
    this.sfx.play(ARCHERY + 'load_arrow.wav', { vol: 0.5 }); this.sfx.play(ARCHERY + 'bow&arrow_pullback.wav', { vol: 0.55 });
  }
  cancelDraw() { this.draw = null; this.vm.setHold(null); }
  releaseBow() {
    const t = this.draw.t, full = Math.min(1, t / this.dur('bow_load', 1.667)); this.draw = null; this.vm.setHold(null);
    if (full < 0.2) return;   // a tap: let down the string
    this.vm.swing('shoot', 0.45); this.self?.play('bow_fire'); this.sfx.play(ARCHERY + 'bow&arrow_shot.wav', { vol: 0.8 }); this.sfx.play(ARCHERY + 'fire_bow.wav', { vol: 0.4 });
    const d = new THREE.Vector3(); this.camera.getWorldDirection(d);
    const from = this.origin().clone().addScaledVector(d, 1.2).add(new THREE.Vector3(0.25, -0.3, 0).applyQuaternion(this.camera.quaternion));
    const m = this.data.missiles.m_Archery_Arrow_01, speed = (m.v0 || 100) * (0.45 + 0.55 * full), range = m.range || 300;
    this.fx.arrow({ from, dir: d.clone(), speed, range }).then(a => { this.lastShot = a; });
    this.net?.shoot({ dir: [d.x, d.y, d.z], from: [from.x, from.y, from.z], power: full, clip: 'bow_fire', dur: this.dur('bow_fire', 0.83) });
  }
  /** staff / thrown: the projectile leaves the weapon */
  cast() {
    const d = new THREE.Vector3(); this.camera.getWorldDirection(d); const from = this.origin().clone().addScaledVector(d, 1.0).add(new THREE.Vector3(0.3, -0.25, 0).applyQuaternion(this.camera.quaternion));
    if (this.w?.group === 'staff') { this.lastShot = this.fx.bolt({ from, dir: d.clone() }); this.net?.shoot({ dir: [d.x, d.y, d.z], from: [from.x, from.y, from.z], power: 1, spell: true, clip: 'magic_fire_01', dur: 0.8 }); }
  }

  // ---- server events (see net handlers in multiplayer.js)
  /** a blow landed: { a: attacker id, v: 'm' | 'p', i: victim id, dmg, hp, max, grp, blocked, at: [x, y, z], killed } */
  onHit(m) {
    const at = new THREE.Vector3(m.at[0], m.at[1], m.at[2]); const gh = this.world.groundHeight(at.x, at.z, at.y + 5); if (m.snap && gh != null && gh > -1e8) at.y = gh;
    if (m.a === this.selfId && !m.am) this.hitMarker(!!m.killed);
    if (m.blocked) { this.sfx.play(`/weapons/parry/df_weapon_parry_${PARRY[m.grp] || 'one_handed'}_1.wav`, { at, min: 5, max: 50 }); return; }
    this.fx.play(m.grp === 'bow' ? 'se_arrow_impact' : m.grp === 'staff' ? 'se_impact_fire' : 'se_impact_' + (IMPACT[m.grp] || 'slashing'), at);
    if (m.killed) this.fx.play('se_bloodsplatter0' + (1 + ((Math.random() * 3) | 0)), at);
    if (m.v === 'p' && m.self) { this.hp.hp = m.hp; this.hp.max = m.max; this.refreshUI(); const f = this.$('hurt'); if (f) { f.classList.add('on'); setTimeout(() => f.classList.remove('on'), 160); } }
  }
  /** feedback for a blow of ours that landed: the crosshair flashes red (and a short thud for the shooter, who may be far from the victim) */
  hitMarker(kill) {
    const c = this.$('cross'); if (c) { c.classList.remove('hit', 'kill'); void c.offsetWidth; c.classList.add(kill ? 'kill' : 'hit'); clearTimeout(this._hm); this._hm = setTimeout(() => c.classList.remove('hit', 'kill'), 220); }
    this.sfx.play('/damage/df_damage_impact_1.wav', { vol: kill ? 0.5 : 0.3, rate: kill ? 0.8 : 1.4 });
  }
  setHp(hp, max) { this.hp = { hp, max }; this.refreshUI(); }
  died(self) { this.dead = true; this.cancelDraw(); this.vm.setHold(null); this.toast('You died'); const d = this.$('death'); if (d) d.classList.add('on'); this.refreshUI(); }
  respawned() { this.dead = false; const d = this.$('death'); if (d) d.classList.remove('on'); this.refreshUI(); }

  // ---- per frame
  update(dt, { speed = 0, air = false } = {}) {
    if (this.draw) this.draw.t += dt;
    const L = this.lightSrc; if (L) this.vm.light(L.sun, L.hemi);
    this.vm.visible = this.drawn && !this.third && !!this.w && !this.dead; this.vm.update(dt, speed, air);
  }
  resize(aspect) { this.vm.resize(aspect); }

  // ---- UI: hotbar + hp + armory
  buildUI() {
    const css = document.createElement('style'); css.textContent = `
      #wbar{position:fixed;left:50%;bottom:14px;transform:translateX(-50%);display:none;flex-direction:column;align-items:center;gap:5px;pointer-events:none;text-shadow:0 1px 3px #000;z-index:6}
      body.playing #wbar{display:flex}
      #wname{color:var(--gold,#d9b86a);font-size:15px;letter-spacing:.05em}
      #hpbar{width:240px;height:9px;background:#0009;border:1px solid #000a;border-radius:2px;overflow:hidden}#hpfill{height:100%;background:linear-gradient(#c43,#722);width:100%;transition:width .15s}
      #hotbar{display:flex;gap:3px}#hotbar i{font-style:normal;min-width:54px;padding:2px 4px;background:#000a;border:1px solid #554;border-radius:3px;color:#bba;font-size:11px;text-align:center;white-space:nowrap}#hotbar i.on{border-color:var(--gold,#d9b86a);color:#fff;background:#3a3020cc}
      #cross.hit{background:#f33!important;box-shadow:0 0 10px 4px #f33;transform:scale(2.2)}#cross.kill{background:#fa0!important;box-shadow:0 0 14px 6px #fa0;transform:scale(3)}
      #hurt{position:fixed;inset:0;pointer-events:none;box-shadow:inset 0 0 120px 30px #a00;opacity:0;transition:opacity .25s;z-index:5}#hurt.on{opacity:.8;transition:none}
      #death{position:fixed;inset:0;pointer-events:none;background:#400a;display:none;align-items:center;justify-content:center;color:#d33;font-size:54px;letter-spacing:.1em;z-index:7}#death.on{display:flex}
      #armory{position:fixed;left:50%;top:50%;transform:translate(-50%,-50%);width:min(720px,92vw);height:min(560px,80vh);background:#14110fee;border:1px solid #554;border-radius:6px;display:none;flex-direction:column;z-index:30;font:13px Palatino,Georgia,serif;color:#ddd}#armory.on{display:flex}
      #armory header{padding:8px 10px;display:flex;gap:8px;align-items:center;border-bottom:1px solid #333}#armory header b{color:var(--gold,#d9b86a);font-weight:normal;letter-spacing:.08em}#armory input{flex:1;background:#221f1b;border:1px solid #444;color:#ddd;padding:4px 6px}
      #armlist{overflow:auto;flex:1;padding:4px 10px 10px}#armlist h4{margin:10px 0 3px;color:var(--gold,#d9b86a);font-weight:normal;font-size:11px;letter-spacing:.08em;text-transform:uppercase}
      #armlist a{display:inline-block;margin:2px 3px 2px 0;padding:2px 7px;border:1px solid #3a352c;border-radius:3px;cursor:pointer;color:#bbb}#armlist a:hover{background:#2f2a22;color:#fff}#armlist a.sel{border-color:var(--gold,#d9b86a);color:#fff}`; document.head.appendChild(css);
    const bar = document.createElement('div'); bar.id = 'wbar'; bar.innerHTML = '<div id="wname"></div><div id="hpbar"><div id="hpfill"></div></div><div id="hotbar"></div>'; document.body.appendChild(bar);
    for (const id of ['hurt', 'death']) { const d = document.createElement('div'); d.id = id; if (id === 'death') d.textContent = 'YOU DIED'; document.body.appendChild(d); }
    const ar = document.createElement('div'); ar.id = 'armory'; ar.innerHTML = '<header><b>ARMORY</b><input id="armq" placeholder="filter weapons (name, class, rank)" autocomplete="off"><span style="color:#887">I to close</span></header><div id="armlist"></div>'; document.body.appendChild(ar);
    ar.querySelector('#armq').oninput = () => this.drawArmory(); ar.querySelector('#armlist').onclick = e => { const id = e.target.closest('a')?.dataset.id; if (id) { this.equip(id); this.toggleArmory(false); } };
    this.refreshUI();
  }
  toggleArmory(on) { const a = this.$('armory'); on ??= !a.classList.contains('on'); a.classList.toggle('on', on); if (on) { document.exitPointerLock?.(); this.drawArmory(); setTimeout(() => this.$('armq').focus(), 30); } else this.$('armq').blur(); return on; }
  drawArmory() {
    const f = this.$('armq').value.toLowerCase(), W = this.data.weapons, groups = {};
    for (const [id, w] of Object.entries(W)) { if (!w.mesh || w.natural || (w.group === 'shield')) continue; if (this.owned && !this.owned.has(id) && id !== this.id) continue; if (f && !(`${w.name} ${w.group} ${GROUP_NAMES[w.group] || ''} ${w.rank || ''}`).toLowerCase().includes(f)) continue; (groups[w.group] ||= []).push([id, w]); }
    this.$('armlist').innerHTML = Object.keys(GROUP_NAMES).filter(g => groups[g]).map(g => `<h4>${GROUP_NAMES[g]}</h4>` + groups[g].sort((a, b) => (a[1].rank || 0) - (b[1].rank || 0)).map(([id, w]) => `<a data-id="${id}" class="${id === this.id ? 'sel' : ''}" title="${(w.desc || '').replace(/"/g, '&quot;')}">${w.name}${w.rank ? ' <small style="color:#887">' + w.rank + '</small>' : ''}</a>`).join('')).join('');
  }
  refreshUI() {
    const w = this.w; const n = this.$('wname'); if (n) n.textContent = w ? `${w.name}${this.drawn ? '' : ' (sheathed)'}` : 'Unarmed  [I] armory · [1-0] weapons';
    const hp = this.$('hpfill'); if (hp) hp.style.width = (100 * Math.max(0, this.hp.hp) / this.hp.max) + '%';
    const hb = this.$('hotbar'); if (hb) hb.innerHTML = this.hotbar.map((id, i) => `<i class="${id === this.id ? 'on' : ''}">${(i + 1) % 10} ${GROUP_NAMES[this.data.weapons[id].group].replace(/s$/, '')}</i>`).join('');
  }
  key(code) {
    if (code === 'KeyI') return this.toggleArmory(), true;
    if (code === 'KeyR') return this.toggleDrawn(), true;
    const m = /^(?:Digit|Numpad)(\d)$/.exec(code); if (m) { const i = (+m[1] + 9) % 10, id = this.hotbar[i]; if (id) this.equip(id); return true; }
    return false;
  }
}
