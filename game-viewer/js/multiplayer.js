// Multiplayer client for game-server (see ../../game-server/src/proto.rs for the wire format).
//   - model picker on the intro screen (race x gender, live rotating preview)
//   - sends our position at 15 Hz, renders everybody the server reports (interpolated, walk-cycle driven by their speed)
//   - name tags + chat bubbles over heads, a chat log (Enter), a radar and an online list (Tab)
import * as THREE from 'three';
import { AvatarFactory, RACES, AVATAR_HEIGHT, pose } from './avatars.js';
import { MobWorld, OP_MOBS } from './mobs.js';
import { PlayerAnim } from './playeranim.js';

const OP_POS = 1, OP_SNAPSHOT = 2, FLAG_SWIM = 1, FLAG_AIR = 2, FLAG_SNAP = 4, FLAG_FLY = 32, FLAG_CROUCH = 128;
const SEND_HZ = 15, INTERP = 0.13;                  // render remote players this many seconds in the past (a bit over one tick)
const AVATAR_NEAR = 420, NAME_NEAR = 260, DOT_FAR = 1500, SHADOW_NEAR = 90, POSE_NEAR = 180;
const RUN = 3.7 * 1.5;                              // m/s of a full-speed stride (matches SPEEDS.run)
const el = (tag, cls, html) => { const e = document.createElement(tag); if (cls) e.className = cls; if (html != null) e.innerHTML = html; return e; };
const esc = s => s.replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const lerpAngle = (a, b, t) => a + Math.atan2(Math.sin(b - a), Math.cos(b - a)) * t;

function textSprite(draw, w, h, order) {
  const c = document.createElement('canvas'); c.width = w; c.height = h; draw(c.getContext('2d'), w, h);
  const t = new THREE.CanvasTexture(c); t.colorSpace = THREE.SRGBColorSpace; t.anisotropy = 4;
  const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: t, transparent: true, depthWrite: false, fog: false }));
  s.renderOrder = order; s.userData.aspect = w / h; return s;
}
/** Name tag: gold for a vendor or a trader, teal for any other NPC. */
function nameSprite(name, npc = false, role = '') {
  const f = '600 30px Palatino, Georgia, serif', m = document.createElement('canvas').getContext('2d'); m.font = f;
  const w = Math.ceil(m.measureText(name).width) + 24;
  return textSprite((x, w, h) => { x.font = f; x.textAlign = 'center'; x.textBaseline = 'middle'; x.fillStyle = '#0007'; x.beginPath(); x.roundRect(0, 6, w, h - 12, 12); x.fill(); x.fillStyle = role ? '#ffd36b' : npc ? '#a6d9cb' : '#f1dfae'; x.fillText(name, w / 2, h / 2 + 1); }, w, 56, 20);
}
function bubbleSprite(text) {
  const f = '26px Palatino, Georgia, serif', m = document.createElement('canvas').getContext('2d'); m.font = f;
  const lines = [], words = text.split(/\s+/); let cur = '';
  for (const wd of words) { const t = cur ? cur + ' ' + wd : wd; if (m.measureText(t).width > 360 && cur) { lines.push(cur); cur = wd; } else cur = t; }
  lines.push(cur); const shown = lines.slice(0, 5);
  const w = Math.min(400, Math.ceil(Math.max(...shown.map(l => m.measureText(l).width))) + 28), h = shown.length * 34 + 26;
  return textSprite((x, w, h) => { x.fillStyle = '#f4efe2ee'; x.beginPath(); x.roundRect(0, 0, w, h - 12, 12); x.fill(); x.beginPath(); x.moveTo(w / 2 - 9, h - 12); x.lineTo(w / 2 + 9, h - 12); x.lineTo(w / 2, h - 1); x.fill();
    x.font = f; x.fillStyle = '#1a1510'; x.textBaseline = 'middle'; shown.forEach((l, i) => x.fillText(l, 14, 17 + 13 + i * 34 - 8)); }, w, h, 21);
}
function dotSprite() {
  return textSprite((x, w, h) => { x.fillStyle = '#ffd98a'; x.strokeStyle = '#000a'; x.lineWidth = 6; x.beginPath(); x.arc(w / 2, h / 2, w / 2 - 6, 0, 7); x.stroke(); x.fill(); }, 32, 32, 19);
}

export class Multiplayer {
  /** `q` = URLSearchParams, `getSelf()` -> { x, y(feet), z, yaw, swim, air } in three.js space */
  constructor({ scene, camera, world, getSelf, toast, $ }) {
    Object.assign(this, { scene, camera, world, getSelf, toast, $ });
    const q = new URLSearchParams(location.search);
    this.url = q.get('server') || `ws://${location.hostname || 'localhost'}:1338`;
    this.factory = new AvatarFactory(); this.mobs = new MobWorld({ scene, camera, world });
    this.players = new Map(); this.shots = new Map(); this.corpses = new Map(); this.me = null; this.ws = null; this.sendAcc = 0; this.last = null; this.speed = 0; this.dotMat = null;
    let saved = {}; try { saved = JSON.parse(localStorage.getItem('dfProfile') || '{}'); } catch {}
    this.profile = { name: q.get('name') || saved.name || '', race: q.get('race') || saved.race || 'hum', gender: q.get('gender') || saved.gender || 'm', gear: q.get('gear') ?? saved.gear ?? '' };
    this.dotTex = null; this.started = false; this.typing = false; this.radius = 900;
    // combat messages we send (see weapons.js; the server decides what they hit, game-server/src/world.rs)
    this.weapons = this.fx = this.sfx = null; this.onRespawn = null;
    this.combat = {
      equip: (w, drawn) => this.sendJson({ t: 'equip', w, drawn }), attack: ({ kind, clip }) => this.sendJson({ t: 'attack', kind, clip }), block: on => this.sendJson({ t: 'block', on }),
      bow: what => this.sendJson({ t: 'bow', what }), shoot: ({ dir, from, power, spell }) => this.sendJson({ t: 'shoot', dir, from, power, spell: !!spell }),
    };
    this.buildPicker(); this.buildChat(); this.buildRadar();
  }

  // ---------------------------------------------------------------- picker (intro screen)
  buildPicker() {
    const $ = this.$, box = $('mp'), P = this.profile;
    box.innerHTML = `<canvas id="mpprev" width="190" height="262"></canvas><div class="f"><div class="t">YOUR CHARACTER</div>
      <input id="mpname" maxlength="16" placeholder="Name" spellcheck="false" autocomplete="off"><div class="t">RACE</div><div id="mprace" class="opts"></div>
      <div class="t">BODY</div><div id="mpgender" class="opts"></div><div class="t">ARMOUR</div><div id="mpgear" class="opts"></div><div id="mpstat"></div></div>`;
    box.addEventListener('click', e => e.stopPropagation());
    box.addEventListener('keydown', e => e.stopPropagation());
    const name = $('mpname'); name.value = P.name; name.addEventListener('input', () => { P.name = name.value; this.saveProfile(); });
    const opts = (id, list, key) => { const c = $(id); c.innerHTML = ''; list.forEach(([v, label]) => { const b = el('div', 'o' + (P[key] === v ? ' on' : ''), label); b.addEventListener('click', () => { P[key] = v; [...c.children].forEach(x => x.classList.toggle('on', x === b)); this.saveProfile(); if (key !== 'gear') this.gearRow(); this.refreshPreview(); }); c.appendChild(b); }); };
    this.opts = opts;
    opts('mprace', RACES.map(r => [r.id, r.name]), 'race'); opts('mpgender', [['m', 'Male'], ['f', 'Female']], 'gender'); this.gearRow();
    // live preview in its own small renderer
    const cv = $('mpprev'); this.pr = new THREE.WebGLRenderer({ canvas: cv, antialias: true, alpha: true }); this.pr.setPixelRatio(Math.min(devicePixelRatio, 2)); this.pr.setSize(190, 262, false);
    cv.style.width = '190px'; cv.style.height = '262px'; this.pr.outputColorSpace = THREE.SRGBColorSpace;
    this.ps = new THREE.Scene(); this.pc = new THREE.PerspectiveCamera(28, 190 / 262, 0.5, 50); this.pc.position.set(0, AVATAR_HEIGHT * 0.52, 7.4);
    this.ps.add(new THREE.HemisphereLight(0xfff2dc, 0x6a5a48, 3.2)); const d = new THREE.DirectionalLight(0xffe2b0, 3.2); d.position.set(2, 4, 6); this.ps.add(d);
    this.pav = null; this.previewLoop(); this.refreshPreview();
  }
  /** armour choices depend on race/gender (the game data defines which sets exist for each) */
  async gearRow() {
    const P = this.profile, list = await this.factory.gearFor(P.race, P.gender);
    if (P.gear && !list.includes(P.gear)) P.gear = '';
    this.opts('mpgear', [['', 'None'], ...list.map(n => [n, n])], 'gear');
  }
  saveProfile() { try { localStorage.setItem('dfProfile', JSON.stringify(this.profile)); } catch {} }
  async refreshPreview() {
    const k = (this.pk = (this.pk || 0) + 1), { race, gender } = this.profile;
    try {
      const av = await this.factory.build(race, gender, this.profile.gear);
      if (k !== this.pk) return;
      if (this.pav) this.ps.remove(this.pav.root);
      this.pav = av; av.root.position.y = -0.05; this.ps.add(av.root);
    } catch (e) { console.warn('preview', e); }
  }
  previewLoop() {
    if (this.started) { this.pr.dispose(); return; }
    requestAnimationFrame(() => this.previewLoop());
    if (this.pav) { this.pav.root.rotation.y += 0.012; pose(this.pav, 0, 0, false); }
    this.pr.render(this.ps, this.pc);
  }

  // ---------------------------------------------------------------- network
  connect() {
    this.started = true; this.$('mpstat').textContent = '';
    if (!this.profile.name.trim()) this.profile.name = '';
    this.open();
  }
  open() {
    clearTimeout(this.retry);
    let ws; try { ws = new WebSocket(this.url); } catch { return this.status('Server address invalid'); }
    ws.binaryType = 'arraybuffer'; this.ws = ws; this.status('Connecting…');
    ws.onopen = () => ws.send(JSON.stringify({ t: 'join', name: this.profile.name, race: this.profile.race, gender: this.profile.gender, gear: this.profile.gear }));
    ws.onmessage = e => typeof e.data === 'string' ? this.onJson(JSON.parse(e.data)) : this.onBinary(e.data);
    ws.onclose = () => { if (this.ws !== ws) return; this.ws = null; this.dropAll(); if (this.fatal) return; this.status('Offline — retrying…'); this.retry = setTimeout(() => this.open(), 3000); };
    ws.onerror = () => {};
  }
  /** Connected players (the server's NPCs share the player roster but are not counted). */
  humans() { let n = 0; for (const p of this.players.values()) if (!p.info.npc) n++; return n; }
  status(s) { this.online = s === 'online'; const e = this.$('mpnet'); e.textContent = this.online ? `● ${this.humans() + 1} online` : `○ ${s}`; e.classList.toggle('up', this.online); }
  dropAll() { for (const c of this.corpses.values()) this.scene.remove(c.root); this.corpses.clear(); for (const p of this.players.values()) this.removeAvatar(p); this.players.clear(); this.me = null; this.mobs.clear(); }

  onJson(m) {
    if (m.t === 'welcome') {
      this.dropAll(); this.me = m.id; this.radius = m.radius;
      for (const p of m.players) if (p.id !== m.id) this.players.set(p.id, { info: p, states: [], av: null, seen: 0 });
      this.last = null; this.status('online'); if (this.weapons) { this.weapons.setHp(120, 120); this.weapons.selfId = this.me; if (this.weapons.id) this.combat.equip(this.weapons.id, this.weapons.drawn); } this.log(`Connected as <b>${esc(m.players.find(p => p.id === m.id)?.name || '')}</b> — ${m.players.filter(p => !p.npc).length} online. Enter to chat, Tab for the player list.`, 'sys');
    } else if (m.t === 'joined') {
      if (m.player.id !== this.me) { this.players.set(m.player.id, { info: m.player, states: [], av: null, seen: 0 }); this.status('online'); this.log(`${esc(m.player.name)} joined`, 'sys'); }
    } else if (m.t === 'left') {
      const p = this.players.get(m.id); if (p) { this.removeAvatar(p); this.players.delete(m.id); this.status('online'); this.log(`${esc(p.info.name)} left`, 'sys'); }
    } else if (m.t === 'chat') {
      const p = this.players.get(m.id); this.log(`<b${m.id === this.me ? ' class="me"' : ''}>${esc(m.name)}</b>: ${esc(m.text)}`);
      if (p) this.say(p, m.text);
    } else if (m.t === 'mobtypes') { this.mobs.setTypes(m.types);
    } else if (this.weapons && this.onCombat(m)) {
    } else if (m.t === 'error') { this.fatal = true; this.ws?.close(); this.status(m.msg); }
  }
  /** Combat events from the server (game-server/src/proto.rs ServerMsg): other players' equip / swings / shots, blows landing, deaths. true = handled */
  onCombat(m) {
    const W = this.weapons, now = performance.now() / 1000, p = this.players.get(m.id), at = a => new THREE.Vector3(a[0], a[1], a[2]);
    if (m.t === 'corpse') { this.addCorpse(m); return true; }
    if (m.t === 'tombstone') { this.addTomb(m); return true; }
    if (m.t === 'looted') { const c = this.corpses.get(m.cid); if (c) { if (c.root) this.scene.remove(c.root); this.corpses.delete(m.cid); } return true; }
    if (m.t === 'shop') { this.shop = m; this.log(`${esc(m.name)} — ${m.goods.length} goods for sale, wants ${m.buys.length} of yours (${m.open ? 'open' : 'shut'})`, 'sys'); return true; }
    if (m.t === 'trade') { this.log(m.ok ? `${m.op === 'buy' ? 'Bought' : 'Sold'} ${m.units} × ${esc(m.item.replace(/^r_/, ''))} for ${Math.round(m.gold)} gold` : `Cannot ${m.op === 'shop' ? 'open the shop' : m.op}: ${esc(m.reason)}`, 'sys'); return true; }
    if (m.t === 'drop') { this.log(`Loot: ${Math.round(m.gold)} gold${m.items.length ? ' and ' + m.items.map(([k, q]) => q + ' × ' + esc(k.replace(/^r_/, ''))).join(', ') : ''}`, 'sys'); return true; }
    if (m.t === 'looting') { this.log(`Looting… stand still for ${m.secs} s`, 'sys'); return true; }
    if (m.t === 'lootend') { if (!m.ok) this.log('Looting interrupted', 'sys'); return true; }
    if (m.t === 'stock') { this.weapons?.setOwned(m.items, m.own); this.stockMsg = m; const g = Math.round(m.gold), n = Object.values(m.items).reduce((a, b) => a + b, 0), had = this.stock; this.stock = { gold: g, units: n, items: m.items }; if (had && (g > had.gold || n > had.units)) this.log(`Looted — you carry ${g} gold and ${n} goods`, 'sys'); else if (had && g === 0 && n === 0 && (had.gold || had.units)) this.log('You lost everything you carried', 'sys'); return true; }
    if (m.t === 'corpsegone') { const c = this.corpses.get(m.cid); if (c) { this.scene.remove(c.root); this.corpses.delete(m.cid); } return true; }
    if (m.t === 'equip' && m.id === this.me) { this.weapons?.equip(m.w, m.drawn, true); return true; }   // the server says what is in our hand
    if (m.t === 'equip') { if (p) { p.info.weapon = m.w; p.info.drawn = m.drawn; this.wield(p); } return true; }
    if (m.t === 'swing') {
      const a = p?.av; if (!a) return true; a.anim?.play(m.clip);
      const g = W.data.groups[m.group]; if (g?.sounds) setTimeout(() => this.sfx.play(this.sfx.pick(g.sounds), { at: a.pos.clone().setY(a.pos.y + 1.5), min: 2, max: 30, vol: 0.9 }), 180);
      return true;
    }
    if (m.t === 'block') { const a = p?.av; if (a?.anim && m.on) a.anim.play(`${({ polearm: 'halberd', knife: '2daggers' })[W.data.weapons[p.info.weapon]?.group] || (/^great/.test(W.data.weapons[p.info.weapon]?.group) ? '2handed' : 'singlehand')}_parrying`); return true; }
    if (m.t === 'bow') { const a = p?.av; if (a) { a.anim?.play('bow_load'); this.sfx.play('/weapons/archery/df_weapon_archery_bow&arrow_pullback.wav', { at: a.pos.clone().setY(a.pos.y + 1.5), min: 3, max: 40, vol: 0.6 }); } return true; }
    if (m.t === 'shoot') {
      const from = at(m.from), dir = at(m.dir), a = p?.av;
      if (m.spell) this.shots.set(m.id, this.fx.bolt({ from, dir }));
      else { this.sfx.play('/weapons/archery/df_weapon_archery_bow&arrow_shot.wav', { at: from, min: 3, max: 60, vol: 0.8 }); this.fx.arrow({ from, dir, speed: 100 * (0.45 + 0.55 * m.power) }).then(s => this.shots.set(m.id, s)); }
      a?.anim?.play(m.spell ? 'magic_fire_01' : 'bow_fire'); return true;
    }
    if (m.t === 'hit') {
      const self = m.v === 'p' && m.i === this.me, shot = this.shots.get(m.a); if (shot && !m.am) { shot.done = true; this.shots.delete(m.a); }
      if (m.a === this.me && !m.am && W.lastShot) W.lastShot.done = true;
      if (m.v === 'p' && m.i !== this.me) { const a = this.players.get(m.i)?.av; if (a?.attached) { m.at = [a.pos.x, a.pos.y, a.pos.z]; m.snap = false; } }   // the avatar stands on the real terrain (bots report no height)
      W.onHit({ ...m, self }); if (m.v === 'm') this.mobs.showHp(m.i, m.hp, m.max, now);
      return true;
    }
    if (m.t === 'died') {
      if (m.v === 'p') { if (m.i === this.me) W.died(); else { const q = this.players.get(m.i); if (q) q.dead = true; } }
      return true;
    }
    if (m.t === 'respawn') {
      if (m.id === this.me) { W.respawned(); W.setHp(m.hp, m.max); this.onRespawn?.(); } else { const q = this.players.get(m.id); if (q) q.dead = false; if (q?.av?.anim) q.av.anim.dead = false; }
      return true;
    }
    return false;
  }
  /** A body on the ground (server: Corpse, kept 10 minutes): the dying clip of the player's own race, held at its last frame. Static, so it costs nothing per frame. */
  async addCorpse(m) {
    if (this.corpses.has(m.cid)) return;
    const c = { root: null, x: m.x, y: m.y, z: m.z, yaw: m.yaw, snap: m.snap, shown: false, gyT: -9, gold: m.gold || 0, units: m.units || 0 }; this.corpses.set(m.cid, c);
    try {
      const av = await this.factory.build(m.race, m.gender, m.gear); if (!this.corpses.has(m.cid)) return;
      const an = await PlayerAnim.create(av).catch(() => null); if (an) { an.dead = true; an.update(0.01); an.update(10); }   // pose: lying where it fell
      av.root.rotation.y = m.yaw + Math.PI; c.root = av.root; c.av = av;
    } catch (e) { console.warn('corpse', e); }
  }
  /** A body turned into its tombstone (the game's own model, tombstone_player_characters_01): what is still on it is lootable with F. */
  async addTomb(m) {
    const old = this.corpses.get(m.cid); if (old?.root) this.scene.remove(old.root);   // the body goes
    const c = { root: null, x: m.x, y: m.y, z: m.z, yaw: m.yaw, snap: m.snap, shown: false, gyT: -9, gold: m.gold || 0, units: m.units || 0, tomb: true, name: m.name }; this.corpses.set(m.cid, c);
    try {
      const src = await this.factory.static('tombstone_player_characters_01'); if (!src || this.corpses.get(m.cid) !== c) return;
      const g = src.clone(true); g.scale.setScalar(1.5); g.rotation.y = m.yaw; g.traverse(o => { if (o.isMesh) { o.castShadow = true; o.receiveShadow = true; } }); c.root = g;
    } catch (e) { console.warn('tombstone', e); }
  }
  /** put the player's weapon into his avatar's hand (and pick the matching stance clips) */
  async wield(p) {
    const a = p.av; if (!a || !this.weapons) return; const w = this.weapons.data.weapons[p.info.weapon] || null;
    await this.factory.wield(a, w && p.info.drawn ? w : null); a.anim?.setWeapon(w?.group || null); a.anim?.setDrawn(!!(w && p.info.drawn));
  }
  onBinary(buf) {
    const v = new DataView(buf); if (v.getUint8(0) === OP_MOBS) return this.mobs.onFrame(buf); if (v.getUint8(0) !== OP_SNAPSHOT) return;
    const n = v.getUint16(5, true), now = performance.now() / 1000;
    for (let i = 0, o = 7; i < n; i++, o += 20) {
      const p = this.players.get(v.getUint16(o, true)); if (!p) continue;
      const s = { t: now, x: v.getFloat32(o + 2, true), y: v.getFloat32(o + 6, true), z: v.getFloat32(o + 10, true), yaw: v.getFloat32(o + 14, true), speed: v.getUint8(o + 18) / 10, flags: v.getUint8(o + 19) };
      p.states.push(s); if (p.states.length > 4) p.states.shift(); p.seen = now;
    }
  }
  sendPos(dt) {
    if (!this.online || this.ws?.readyState !== 1) return;
    const s = this.getSelf(), b = new DataView(new ArrayBuffer(19)); b.setUint8(0, OP_POS);
    b.setFloat32(1, s.x, true); b.setFloat32(5, s.y, true); b.setFloat32(9, s.z, true); b.setFloat32(13, s.yaw, true);
    if (this.last) this.speed += (Math.hypot(s.x - this.last.x, s.z - this.last.z) / dt - this.speed) * 0.5;   // smoothed horizontal speed
    this.last = s;
    b.setUint8(17, Math.min(255, Math.round(this.speed * 10))); b.setUint8(18, (s.swim ? FLAG_SWIM : 0) | (s.air ? FLAG_AIR : 0) | (s.fly ? FLAG_FLY : 0) | (s.crouch ? FLAG_CROUCH : 0));
    this.ws.send(b.buffer);
  }
  sendJson(o) { if (this.online && this.ws?.readyState === 1) this.ws.send(JSON.stringify(o)); }
  bindCombat(weapons, fx, sfx) { this.weapons = weapons; this.fx = fx; this.sfx = sfx; }
  chat(text) { if (this.ws?.readyState === 1 && text.trim()) this.ws.send(JSON.stringify({ t: 'chat', text })); }

  // ---------------------------------------------------------------- chat UI
  buildChat() {
    const $ = this.$; this.logEl = $('chatlog'); this.input = $('chatin');
    addEventListener('keydown', e => {
      if (!document.body.classList.contains('playing')) return;
      if (e.code === 'KeyF' && !e.repeat && !this.typing) { this.lootNearest(); return; }
      if (e.code === 'KeyB' && !e.repeat && !this.typing) { this.openShop(); return; }
      if (e.code === 'Tab') { e.preventDefault(); if (!e.repeat) { $('plist').classList.toggle('on'); this.renderList(); } return; }
      if (!this.typing && (e.code === 'Enter' || e.code === 'NumpadEnter') && !e.repeat) { e.preventDefault(); this.openChat(); }
    });
    this.input.addEventListener('keydown', e => {
      e.stopPropagation();
      if (e.code === 'Enter' || e.code === 'NumpadEnter') { this.chat(this.input.value); this.closeChat(); }
      else if (e.code === 'Escape') this.closeChat();
    });
  }
  /** F: take everything from the nearest body with something on it within reach (the server checks the distance too). */
  lootNearest() {
    const cam = this.camera.position; let best = null, bd = 6;
    for (const [cid, c] of this.corpses) { if (!c.tomb || !(c.gold > 0 || c.units > 0)) continue; const d = Math.hypot(c.x - cam.x, c.z - cam.z); if (d < bd) { bd = d; best = cid; } }
    if (best !== null) this.sendJson({ t: 'loot', cid: best });
  }
  /** The vendor NPC nearest to us within reach (the server decides in the end), for a trading window. */
  nearestVendor(reach = 8) {
    const cam = this.camera.position; let best = null, bd = reach;
    for (const p of this.players.values()) { if (p.info.role !== 'vendor' || !p.av?.root) continue; const d = Math.hypot(p.av.root.position.x - cam.x, p.av.root.position.z - cam.z); if (d < bd) { bd = d; best = p.info.id; } }
    return best;
  }
  openShop(npc = this.nearestVendor()) { if (npc != null) this.sendJson({ t: 'shop', npc }); else this.log('No vendor within reach', 'sys'); }
  buy(npc, item, qty = 1) { this.sendJson({ t: 'buy', npc, item, qty }); }
  sell(npc, item, qty = 1) { this.sendJson({ t: 'sell', npc, item, qty }); }
  openChat() { this.typing = true; document.exitPointerLock?.(); this.$('chat').classList.add('typing'); this.input.value = ''; this.input.focus(); }
  closeChat() { this.typing = false; this.input.blur(); this.$('chat').classList.remove('typing'); this.$('view').requestPointerLock?.()?.catch?.(() => {}); }
  log(html, cls = '') {
    const d = el('div', 'l ' + cls, html); this.logEl.appendChild(d); while (this.logEl.children.length > 60) this.logEl.firstChild.remove();
    this.logEl.scrollTop = 1e9; setTimeout(() => d.classList.add('old'), 14000);
  }

  // ---------------------------------------------------------------- avatars
  async ensureAvatar(p) {
    if (p.av || p.building) return; p.building = true;
    try {
      const av = await this.factory.build(p.info.race, p.info.gender, p.info.gear);
      if (!this.players.has(p.info.id)) return;
      av.tag = nameSprite(p.info.name, p.info.npc, p.info.role); av.dot = dotSprite(); av.pos = new THREE.Vector3(); av.cast = null;
      p.av = av; av.attached = false;
      av.anim = await PlayerAnim.create(av).catch(() => null); if (p.info.weapon) await this.wield(p);
    } catch (e) { console.warn('avatar', p.info, e); p.failed = true; } finally { p.building = false; }
  }
  removeAvatar(p) {
    const a = p.av; if (!a) return;
    this.scene.remove(a.root, a.tag, a.dot); if (a.bubble) { this.scene.remove(a.bubble); a.bubble.material.map.dispose(); a.bubble.material.dispose(); }
    for (const s of [a.tag, a.dot]) { s.material.map.dispose(); s.material.dispose(); }
    p.av = null; p.states.length = 0;
  }
  say(p, text) {
    const a = p.av; if (!a) return;
    if (a.bubble) { this.scene.remove(a.bubble); a.bubble.material.map.dispose(); a.bubble.material.dispose(); }
    a.bubble = bubbleSprite(text); a.bubbleT = performance.now() / 1000 + 3 + Math.min(7, text.length * 0.08); this.scene.add(a.bubble);
  }

  /** Interpolated state at render time `rt`, with the animation inputs of the newer sample. */
  sample(p, rt, out) {
    const S = p.states; if (!S.length) return false;
    let a = S[0], b = S[S.length - 1];
    for (let i = S.length - 1; i > 0; i--) if (S[i - 1].t <= rt) { a = S[i - 1]; b = S[i]; break; }
    const k = b.t > a.t ? Math.min(1, Math.max(0, (rt - a.t) / (b.t - a.t))) : 1;
    out.x = a.x + (b.x - a.x) * k; out.y = a.y + (b.y - a.y) * k; out.z = a.z + (b.z - a.z) * k; out.yaw = lerpAngle(a.yaw, b.yaw, k);
    out.speed = b.speed; out.flags = b.flags; return true;
  }

  update(dt, now) {
    this.sendAcc += dt;
    if (this.sendAcc >= 1 / SEND_HZ) { this.sendPos(this.sendAcc); this.sendAcc = 0; }
    const cam = this.camera.position, rt = now - INTERP, st = {}, fovK = 2 * Math.tan(THREE.MathUtils.degToRad(this.camera.fov / 2)) / innerHeight;
    for (const p of this.players.values()) {
      if (now - p.seen > 1.5) { if (p.av) { this.scene.remove(p.av.root, p.av.tag, p.av.dot); if (p.av.bubble) this.scene.remove(p.av.bubble); p.av.attached = false; p.av.bubble = null; p.states.length = 0; } continue; }   // out of the server's interest radius
      if (!p.av) { this.ensureAvatar(p); continue; }
      if (p.dead) { const d = p.av; if (d.attached) { this.scene.remove(d.root, d.tag); d.attached = false; } if (d.dot.parent) this.scene.remove(d.dot); continue; }   // lying as a corpse (see addCorpse), not walking about
      const a = p.av; if (!this.sample(p, rt, st)) continue;
      if (st.flags & FLAG_SNAP) { const g = this.world.groundHeight(st.x, st.z); a.gy = g != null && g > -1e8 ? g : (a.gy ?? st.y); st.y = a.gy; }
      a.pos.set(st.x, st.y, st.z);
      const dist = a.pos.distanceTo(cam), full = dist < AVATAR_NEAR;
      if (full !== a.attached) { a.attached = full; if (full) this.scene.add(a.root, a.tag); else this.scene.remove(a.root, a.tag); }
      a.dot.visible = !full || dist > NAME_NEAR; a.tag.visible = dist < NAME_NEAR;
      if (a.dot.visible) { if (!a.dot.parent) this.scene.add(a.dot); a.dot.position.set(st.x, st.y + AVATAR_HEIGHT * 0.5, st.z); const h = 11 * fovK * dist; a.dot.scale.set(h, h, 1); }
      else if (a.dot.parent) this.scene.remove(a.dot);
      if (!full) { if (a.bubble) a.bubble.visible = false; continue; }
      a.root.position.set(st.x, st.y, st.z); a.root.rotation.y = st.yaw + Math.PI;   // meshes face +Z, the camera's forward is -Z
      // animation: stride phase advances with distance travelled
      const amp = Math.min(1, st.speed / RUN), air = !!(st.flags & FLAG_AIR) && !(st.flags & FLAG_SWIM);
      if (st.speed > 0.15 && !air) a.phase += dt * (3 + st.speed * 1.9);
      if (dist < POSE_NEAR || !a.posed) { if (a.anim) a.anim.update(dt, st.speed, air, !!(st.flags & FLAG_SWIM), (st.flags & FLAG_CROUCH) ? { crouch: true } : null); else pose(a, Math.sin(a.phase), st.speed > 0.15 ? Math.max(0.25, amp) : 0, air); a.posed = true; }
      const shadow = dist < SHADOW_NEAR; if (shadow !== a.cast) { a.cast = shadow; a.root.traverse(n => { if (n.isMesh) n.castShadow = shadow; }); }
      // tags above the head, constant on-screen size
      const topY = st.y + AVATAR_HEIGHT + 0.25, th = 22 * fovK * dist, ty = th + 0.1;
      a.tag.position.set(st.x, topY, st.z); a.tag.scale.set(th * a.tag.userData.aspect, th, 1); a.tag.center.set(0.5, 0);
      a.tag.material.opacity = Math.min(1, (NAME_NEAR - dist) / 60);
      if (a.bubble) {
        if (now > a.bubbleT || dist > 160) { this.scene.remove(a.bubble); a.bubble.material.map.dispose(); a.bubble.material.dispose(); a.bubble = null; }
        else { a.bubble.visible = true; const bh = 16 * fovK * dist * (a.bubble.material.map.image.height / 34); a.bubble.center.set(0.5, 0); a.bubble.position.set(st.x, topY + ty * 0.9, st.z); a.bubble.scale.set(bh * a.bubble.userData.aspect, bh, 1); }
      }
    }
    for (const c of this.corpses.values()) {
      if (!c.root) continue; const dd = Math.hypot(c.x - cam.x, c.z - cam.z), near = dd < AVATAR_NEAR;
      if (near && now - c.gyT > 1) { c.gyT = now; let y = c.y; if (c.snap) { const g = this.world.groundHeight(c.x, c.z); if (g != null && g > -1e8) y = g; } c.root.position.set(c.x, y, c.z); }
      if (near !== c.shown) { c.shown = near; if (near) this.scene.add(c.root); else this.scene.remove(c.root); }
    }
    this.mobs.update(dt, now); this.drawRadar(); if (this.$('plist').classList.contains('on')) { this.listAcc = (this.listAcc || 0) + dt; if (this.listAcc > 0.5) { this.listAcc = 0; this.renderList(); } }
  }

  // ---------------------------------------------------------------- radar + list
  buildRadar() { const c = this.$('radar'); this.rc = c.getContext('2d'); }
  drawRadar() {
    const c = this.rc, W = 140, R = this.radius, cam = this.camera, yaw = cam.rotation.y, now = performance.now() / 1000;
    c.clearRect(0, 0, W, W); c.fillStyle = '#0a0c10aa'; c.strokeStyle = '#d9b86a66'; c.beginPath(); c.arc(70, 70, 68, 0, 7); c.fill(); c.stroke();
    c.beginPath(); c.arc(70, 70, 34, 0, 7); c.stroke();
    const cs = Math.cos(yaw), sn = Math.sin(yaw);
    for (const p of this.players.values()) {
      const s = p.states[p.states.length - 1]; if (!s || now - p.seen > 1.5 || p.dead) continue;
      const dx = s.x - cam.position.x, dz = s.z - cam.position.z, rx = dx * cs - dz * sn, rz = dx * sn + dz * cs;   // into the camera's frame (up = forward)
      const k = 66 / R, px = 70 + rx * k, py = 70 + rz * k; if (Math.hypot(px - 70, py - 70) > 66) continue;
      c.fillStyle = p.info.role ? '#ffd36b' : p.info.npc ? '#8fd4c0' : '#ffd98a'; c.beginPath(); c.arc(px, py, p.info.npc ? 2 : 2.5, 0, 7); c.fill();
    }
    c.fillStyle = '#d0584a'; this.mobs.each((x, z) => {
      const dx = x - cam.position.x, dz = z - cam.position.z, k = 66 / R, px = 70 + (dx * cs - dz * sn) * k, py = 70 + (dx * sn + dz * cs) * k;
      if (Math.hypot(px - 70, py - 70) <= 66) c.fillRect(px - 1, py - 1, 2, 2);
    });
    c.fillStyle = '#fff'; c.beginPath(); c.moveTo(70, 63); c.lineTo(74, 73); c.lineTo(66, 73); c.fill();
  }
  renderList() {
    const cam = this.camera.position, now = performance.now() / 1000, rows = [];
    for (const p of this.players.values()) {
      if (p.info.npc) continue;
      const s = p.states[p.states.length - 1], vis = s && now - p.seen < 1.5, d = vis ? Math.hypot(s.x - cam.x, s.z - cam.z) : Infinity;
      let dir = ''; if (vis) { const a = Math.atan2(s.x - cam.x, -(s.z - cam.z)); dir = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'][Math.round(((a + 2 * Math.PI) % (2 * Math.PI)) / (Math.PI / 4)) % 8]; }
      rows.push([d, `<div class="r"><span>${esc(p.info.name)}</span><small>${RACES.find(r => r.id === p.info.race)?.name} ${p.info.gender === 'f' ? '♀' : '♂'}</small><i>${vis ? `${dir} ${d < 1000 ? Math.round(d) + ' m' : (d / 1000).toFixed(1) + ' km'}` : 'out of range'}</i></div>`]);
    }
    rows.sort((a, b) => a[0] - b[0]);
    this.$('plist').innerHTML = `<b>ONLINE — ${rows.length + 1}</b><div class="r me"><span>${esc(this.profile.name || 'you')}</span><small>you</small><i></i></div>` + rows.map(r => r[1]).join('') + `<div class="n">Positions are streamed within ${Math.round(this.radius)} m of you. Tab closes this.</div>`;
  }
}
