//! The world task: one task owns every player, so there are no locks. Connection tasks talk to it through `Cmd`s and receive
//! messages through their own bounded channel (a slow client drops position snapshots, and is kicked if it can't keep up with events).
use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;

use crate::combat::*;
use crate::mobs::{MobAttack, Mobs};
use crate::npcs::{LodCounts, Npcs, NPC_ID_BASE};
use crate::proto::*;
use crate::store::Store;

#[derive(Clone, Debug)]
pub struct Config {
    /// Players further apart than this (metres, x/z plane) don't receive each other's position updates.
    pub radius: f32,
    pub max_players: usize,
    pub tick_hz: u32,
    /// Path of the mob spawn JSON (`game-viewer/data/mobspawns.json`); `None` runs without mobs.
    pub spawns: Option<String>,
    /// Path of `game-viewer/data/weapons.json` (stats, clip lengths); `None` = combat off (equip / swing messages are still relayed).
    pub weapons: Option<String>,
    /// Players can hurt each other.
    pub pvp: bool,
    /// Number of living NPCs (0 = none) and the data they live from: `game-viewer/data/pois.json` and `spawns.json` (the racial capitals).
    pub npcs: usize,
    pub pois: Option<String>,
    pub capitals: Option<String>,
    /// Seed of the NPC population.
    pub npc_seed: u32,
    /// SQLite file the NPCs are saved to and restored from; `None` = they start fresh every time.
    pub db: Option<String>,
}

/// Seconds between autosaves of the NPC population.
const SAVE_EVERY: u64 = 30;

/// Mob positions are sent every this many ticks (they walk at ~1 m/s, so clients interpolate).
const MOB_EVERY: u32 = 3;

pub struct JoinReq {
    pub name: String,
    pub race: String,
    pub gender: String,
    pub gear: String,
    pub tx: mpsc::Sender<Message>,
    pub reply: oneshot::Sender<Result<u16, &'static str>>,
}

pub enum Cmd {
    Join(JoinReq),
    Pos(u16, PosUpdate),
    Chat(u16, String),
    Leave(u16),
    Equip(u16, String, bool),
    Attack(u16, String, String),
    Block(u16, bool),
    Bow(u16, String),
    Shoot(u16, [f32; 3], [f32; 3], f32, bool),
    Respawn(u16),
}

/// A blow whose animation has started and that lands at `due`.
struct Pending {
    due: Instant,
    owner: u16,
}

/// How long a body stays on the ground after its owner died.
pub const CORPSE_SECS: u64 = 600;

struct Corpse {
    cid: u32,
    expires: Instant,
    /// The JSON of its `Corpse` message, kept for players who join later.
    msg: String,
}

/// Seconds a dead player lies there before he gets up (a Respawn message may bring it forward after DOWN_MIN).
const DOWN_SECS: f32 = 6.0;
const DOWN_MIN: f32 = 2.5;

struct Player {
    info: PlayerInfo,
    pos: PosUpdate,
    has_pos: bool,
    tx: mpsc::Sender<Message>,
    last_chat: Instant,
    last_sent: usize,
    last_mobs: usize,
    dead: bool,
    // combat
    weapon: String,
    group: String,
    drawn: bool,
    blocking: bool,
    hp: f32,
    down_until: Option<Instant>,
    next_attack: Instant,
}

#[derive(Default)]
struct Stats {
    ticks: u32,
    tick_us_sum: u128,
    tick_us_max: u128,
    bytes_out: u64,
    snapshots_out: u64,
    dropped: u64,
    lod: LodCounts,
}

struct World {
    cfg: Config,
    players: Vec<Option<Player>>,
    online: usize,
    cursor: usize,
    tick: u32,
    mobs: Mobs,
    npcs: Npcs,
    store: Option<Store>,
    stats: Stats,
    weapons: Weapons,
    pending: Vec<Pending>,
    shots: Vec<Projectile>,
    corpses: Vec<Corpse>,
    next_corpse: u32,
}

pub async fn run(cfg: Config, mut rx: mpsc::Receiver<Cmd>) {
    let mobs = match &cfg.spawns {
        Some(path) => Mobs::load(path, cfg.radius).unwrap_or_else(|e| {
            eprintln!("no mobs: {e}");
            Mobs::empty()
        }),
        None => Mobs::empty(),
    };
    println!("{} mobs of {} types", mobs.mobs.len(), mobs.types.len());
    let weapons = match &cfg.weapons {
        Some(path) => Weapons::load(path).unwrap_or_else(|e| {
            eprintln!("no weapon table (combat off): {e}");
            Weapons::default()
        }),
        None => Weapons::default(),
    };
    println!("{} weapons", weapons.defs.len());
    let npcs = match (cfg.npcs, &cfg.pois, &cfg.capitals) {
        (n, Some(pois), Some(caps)) if n > 0 => Npcs::load(pois, caps, n, cfg.npc_seed).unwrap_or_else(|e| {
            eprintln!("no npcs: {e}");
            Npcs::empty()
        }),
        _ => Npcs::empty(),
    };
    println!("{} npcs in {} places", npcs.npcs.len(), npcs.places.len());
    let mut npcs = npcs;
    let store = match (&cfg.db, npcs.npcs.is_empty()) {
        (Some(path), false) => match Store::open(path).and_then(|s| s.load(npcs.places.len()).map(|rows| (s, rows))) {
            Ok((s, rows)) => {
                println!("{path}: restored {} of {} saved npcs", npcs.restore(&rows), rows.len());
                Some(s)
            }
            Err(e) => {
                eprintln!("{path}: {e} (npcs will not be saved)");
                None
            }
        },
        _ => None,
    };
    let mut world = World { players: (0..cfg.max_players.min(NPC_ID_BASE as usize)).map(|_| None).collect(), cfg: cfg.clone(), online: 0, cursor: 0, tick: 0, mobs, npcs, store, stats: Stats::default(), weapons, pending: Vec::new(), shots: Vec::new(), corpses: Vec::new(), next_corpse: 1 };
    let mut ticker = interval(Duration::from_secs_f64(1.0 / cfg.tick_hz as f64));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut report = interval(Duration::from_secs(5));
    report.tick().await;
    let mut autosave = interval(Duration::from_secs(SAVE_EVERY));
    autosave.tick().await;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd { Some(c) => world.handle(c), None => break },
            _ = autosave.tick() => world.save(),
            _ = &mut ctrl_c => break,
            _ = ticker.tick() => {
                let t0 = Instant::now();
                world.step();
                let us = t0.elapsed().as_micros();
                world.stats.ticks += 1;
                world.stats.tick_us_sum += us;
                world.stats.tick_us_max = world.stats.tick_us_max.max(us);
            }
            _ = report.tick() => world.report(),
        }
    }
    world.save();
}

impl World {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Join(req) => self.join(req),
            Cmd::Pos(id, p) => {
                if let Some(Some(pl)) = self.players.get_mut(id as usize) {
                    // a dead player lies where he fell (bots keep "walking": ignored)
                    if pl.down_until.is_none() || !pl.has_pos {
                        pl.pos = p;
                        pl.has_pos = true;
                    }
                }
            }
            Cmd::Chat(id, text) => self.chat(id, &text),
            Cmd::Leave(id) => self.remove(id),
            Cmd::Equip(id, w, drawn) => self.equip(id, w, drawn),
            Cmd::Attack(id, kind, clip) => self.attack(id, &kind, &clip),
            Cmd::Block(id, on) => self.block(id, on),
            Cmd::Bow(id, what) => self.relay(id, |id| serde_json::to_string(&ServerMsg::Bow { id, what: &what }).unwrap()),
            Cmd::Shoot(id, from, dir, power, spell) => self.shoot(id, from, dir, power, spell),
            Cmd::Respawn(id) => self.respawn(id, false),
        }
        self.reap();
    }

    fn join(&mut self, req: JoinReq) {
        if !RACES.contains(&req.race.as_str()) || !GENDERS.contains(&req.gender.as_str()) {
            let _ = req.reply.send(Err("unknown race or gender"));
            return;
        }
        let n = self.players.len();
        let Some(id) = (0..n).map(|i| (self.cursor + i) % n).find(|&i| self.players[i].is_none()) else {
            let _ = req.reply.send(Err("server full"));
            return;
        };
        self.cursor = (id + 1) % n;
        let mut name = clean(&req.name, MAX_NAME);
        if name.is_empty() {
            name = format!("Player{id}");
        }
        let info = PlayerInfo { id: id as u16, name, race: req.race, gender: req.gender, gear: clean(&req.gear, MAX_GEAR), weapon: String::new(), drawn: false, npc: false };
        self.players[id] = Some(Player { info, pos: PosUpdate::default(), has_pos: false, tx: req.tx, last_chat: Instant::now() - Duration::from_secs(10), last_sent: 0, last_mobs: 0, dead: false, weapon: String::new(), group: String::new(), drawn: false, blocking: false, hp: PLAYER_HP, down_until: None, next_attack: Instant::now() });
        self.online += 1;
        let _ = req.reply.send(Ok(id as u16));

        let me = self.players[id].as_ref().unwrap();
        let all: Vec<&PlayerInfo> = self.players.iter().flatten().map(|p| &p.info).chain(self.npcs.infos()).collect();
        let welcome = serde_json::to_string(&ServerMsg::Welcome { id: id as u16, radius: self.cfg.radius, tick_hz: self.cfg.tick_hz, players: all }).unwrap();
        if me.tx.try_send(Message::Text(welcome)).is_err() {
            self.players[id].as_mut().unwrap().dead = true;
            return;
        }
        if !self.mobs.types.is_empty() {
            let types = serde_json::to_string(&ServerMsg::Mobtypes { types: &self.mobs.types }).unwrap();
            if self.players[id].as_ref().unwrap().tx.try_send(Message::Text(types)).is_err() {
                self.players[id].as_mut().unwrap().dead = true;
                return;
            }
        }
        let bodies: Vec<String> = self.corpses.iter().map(|c| c.msg.clone()).collect();
        for b in bodies {
            if self.players[id].as_ref().unwrap().tx.try_send(Message::Text(b)).is_err() {
                self.players[id].as_mut().unwrap().dead = true;
                return;
            }
        }
        let me = self.players[id].as_ref().unwrap();
        let joined = ServerMsg::Joined { player: &me.info };
        let text = serde_json::to_string(&joined).unwrap();
        self.broadcast_text(text, Some(id as u16));
        println!("+ {} ({}_{}) id {} — {} online", self.players[id].as_ref().unwrap().info.name, self.players[id].as_ref().unwrap().info.race, self.players[id].as_ref().unwrap().info.gender, id, self.online);
    }

    // ---------------------------------------------------------------- combat
    /// Send `text` to everybody within the interest radius of (x, z) (players whose position is not known yet get it too).
    fn send_near(&mut self, x: f32, z: f32, text: String, except: Option<u16>) {
        let r2 = self.cfg.radius * self.cfg.radius;
        for p in self.players.iter_mut().flatten() {
            if Some(p.info.id) == except || (p.has_pos && (p.pos.x - x).powi(2) + (p.pos.z - z).powi(2) > r2) {
                continue;
            }
            if p.tx.try_send(Message::Text(text.clone())).is_err() {
                p.dead = true;
            }
        }
    }

    /// Tell the players near `id` something about him (not `id` himself).
    fn relay(&mut self, id: u16, msg: impl FnOnce(u16) -> String) {
        let Some(Some(p)) = self.players.get(id as usize) else { return };
        let (x, z) = (p.pos.x, p.pos.z);
        self.send_near(x, z, msg(id), Some(id));
    }

    fn equip(&mut self, id: u16, w: String, drawn: bool) {
        let w = clean(&w, MAX_WEAPON);
        let group = if w.is_empty() { Some(String::new()) } else if self.weapons.defs.is_empty() { Some("sword".to_string()) } else { self.weapons.get(&w).and_then(|d| d.group.clone()) };
        let Some(group) = group else { return };
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        p.weapon = w.clone();
        p.group = group;
        p.drawn = drawn && !w.is_empty();
        p.blocking = false;
        p.info.weapon = w.clone();
        p.info.drawn = p.drawn;
        let drawn = p.drawn;
        self.relay(id, |id| serde_json::to_string(&ServerMsg::Equip { id, w: &w, drawn }).unwrap());
    }

    fn block(&mut self, id: u16, on: bool) {
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        if p.down_until.is_some() || !p.drawn || !is_melee(&p.group) {
            p.blocking = false;
            return;
        }
        p.blocking = on;
        self.relay(id, |id| serde_json::to_string(&ServerMsg::Block { id, on }).unwrap());
    }

    /// A swing / cast / throw starts. Melee blows land at HIT_AT of the clip (see `combat()`); the swing may not be repeated before its clip is over.
    fn attack(&mut self, id: u16, kind: &str, clip: &str) {
        let now = Instant::now();
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        if p.down_until.is_some() || !p.drawn || p.group.is_empty() || p.group == "shield" || now < p.next_attack {
            return;
        }
        let group = p.group.clone();
        let dur = self.weapons.clip(swing_clip(&group), 1.0);
        p.next_attack = now + Duration::from_secs_f32(dur * 0.9);
        let (kind, clip) = (clean(kind, 12), clean(clip, 40));
        if is_melee(&group) {
            self.pending.push(Pending { due: now + Duration::from_secs_f32(dur * HIT_AT), owner: id });
        }
        self.relay(id, |id| serde_json::to_string(&ServerMsg::Swing { id, kind: &kind, clip: &clip, group: &group }).unwrap());
    }

    /// An arrow or bolt leaves a bow / staff. The shot has to start near the shooter and may not come faster than the weapon allows.
    fn shoot(&mut self, id: u16, from: [f32; 3], dir: [f32; 3], power: f32, spell: bool) {
        let now = Instant::now();
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        let ok_group = if spell { p.group == "staff" || p.group == "thrown" } else { p.group == "bow" };
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        let near = (from[0] - p.pos.x).powi(2) + (from[2] - p.pos.z).powi(2) < 100.0;
        if p.down_until.is_some() || !p.drawn || !ok_group || len < 0.5 || !near || !from.iter().all(|v| v.is_finite()) || !dir.iter().all(|v| v.is_finite()) {
            return;
        }
        if spell && now < p.next_attack.checked_sub(Duration::from_millis(500)).unwrap_or(now) {
            return;
        }
        if !spell {
            if now < p.next_attack {
                return;
            }
            p.next_attack = now + Duration::from_millis(500);
        }
        let group = p.group.clone();
        let Some(def) = self.weapons.get(&p.weapon).cloned().or_else(|| self.weapons.defs.is_empty().then(|| WeaponDef { group: Some(group.clone()), kind: String::new(), reach: None, angle: None, rank: None })) else { return };
        let power = if spell { 1.0 } else { power.clamp(0.2, 1.0) };
        let speed = if spell { 45.0 } else { 100.0 * (0.45 + 0.55 * power) };
        let d = [dir[0] / len, dir[1] / len, dir[2] / len];
        let dmg = damage(&def, &group, power, (self.tick % 7) as f32 / 6.0);
        self.shots.push(Projectile { owner: id, pos: from, vel: [d[0] * speed, d[1] * speed, d[2] * speed], left: if spell { 120.0 } else { 300.0 }, group, dmg, spell });
        self.relay(id, |id| serde_json::to_string(&ServerMsg::Shoot { id, from, dir: d, power, spell }).unwrap());
    }

    fn respawn(&mut self, id: u16, forced: bool) {
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        let Some(until) = p.down_until else { return };
        let min = until - Duration::from_secs_f32(DOWN_SECS - DOWN_MIN);
        if !forced && Instant::now() < min {
            return;
        }
        p.down_until = None;
        p.hp = PLAYER_HP;
        let msg = serde_json::to_string(&ServerMsg::Respawn { id, hp: PLAYER_HP, max: PLAYER_HP }).unwrap();
        self.broadcast_text(msg, None);
    }

    /// Apply a blow to player `vid`: blocking, hit points, death. `from` is where it came from (for the block test), `by` the attacker (id, is a mob).
    fn hurt_player(&mut self, vid: u16, dmg: f32, by: (u16, bool), grp: &str, from: (f32, f32)) {
        let now = Instant::now();
        let Some(Some(p)) = self.players.get_mut(vid as usize) else { return };
        if p.down_until.is_some() {
            return;
        }
        let blocked = p.blocking && blocks(p.pos.yaw, (p.pos.x, p.pos.z), from);
        let dmg = if blocked { (dmg * BLOCK_FACTOR).round().max(1.0) } else { dmg };
        p.hp = (p.hp - dmg).max(0.0);
        let killed = p.hp <= 0.0;
        if killed {
            p.down_until = Some(now + Duration::from_secs_f32(DOWN_SECS));
            p.blocking = false;
        }
        let (at, hp, snap) = ([p.pos.x, p.pos.y, p.pos.z], p.hp, p.pos.flags & FLAG_SNAP != 0);
        let msg = serde_json::to_string(&ServerMsg::Hit { a: by.0, am: by.1, v: "p", i: vid, dmg, hp, max: PLAYER_HP, grp, blocked, at, snap, killed }).unwrap();
        self.send_near(at[0], at[2], msg, None);
        if killed {
            self.send_near(at[0], at[2], serde_json::to_string(&ServerMsg::Died { v: "p", i: vid }).unwrap(), None);
            self.leave_corpse(vid);
        }
    }

    /// The body of a player who just died stays for CORPSE_SECS, visible to everybody.
    fn leave_corpse(&mut self, id: u16) {
        let Some(Some(p)) = self.players.get(id as usize) else { return };
        let cid = self.next_corpse;
        self.next_corpse += 1;
        let msg = serde_json::to_string(&ServerMsg::Corpse { cid, name: &p.info.name, race: &p.info.race, gender: &p.info.gender, gear: &p.info.gear, x: p.pos.x, y: p.pos.y, z: p.pos.z, yaw: p.pos.yaw, snap: p.pos.flags & FLAG_SNAP != 0 }).unwrap();
        self.corpses.push(Corpse { cid, expires: Instant::now() + Duration::from_secs(CORPSE_SECS), msg: msg.clone() });
        self.broadcast_text(msg, None);
    }

    fn hit_mob(&mut self, owner: u16, mid: u16, dmg: f32, grp: &str) {
        let Some((hp, killed)) = self.mobs.damage(mid, dmg, owner) else { return };
        let m = &self.mobs.mobs[mid as usize];
        let (at, max) = ([m.pos.x, 0.0, m.pos.z], m.max_hp);
        let msg = serde_json::to_string(&ServerMsg::Hit { a: owner, am: false, v: "m", i: mid, dmg, hp, max, grp, blocked: false, at, snap: true, killed }).unwrap();
        self.send_near(at[0], at[2], msg, None);
        if killed {
            self.send_near(at[0], at[2], serde_json::to_string(&ServerMsg::Died { v: "m", i: mid }).unwrap(), None);
        }
    }

    /// The blow of `owner`'s swing lands now: the nearest living body inside the weapon's arc takes it.
    fn melee_lands(&mut self, owner: u16) {
        let Some(Some(p)) = self.players.get(owner as usize) else { return };
        if p.down_until.is_some() || !p.drawn || p.pos.flags & FLAG_FLY != 0 {
            return;
        }
        let group = p.group.clone();
        let Some(def) = self.weapons.get(&p.weapon).cloned() else { return };
        let (reach, arc) = (def.reach.unwrap_or(2.1) * WORLD_SCALE, def.angle.unwrap_or(60.0));
        let (a, yaw, ay) = ((p.pos.x, p.pos.z), p.pos.yaw, p.pos.y);
        let tr = BODY_RADIUS * WORLD_SCALE;
        let mut best: Option<(f32, bool, u16)> = None; // (distance, is mob, id)
        for i in self.mobs.candidates(a.0, a.1) {
            let m = &self.mobs.mobs[i as usize];
            let t = (m.pos.x, m.pos.z);
            if m.alive && in_arc(a, yaw, t, reach, arc, tr) {
                let d = ((t.0 - a.0).powi(2) + (t.1 - a.1).powi(2)).sqrt();
                if best.map_or(true, |b| d < b.0) {
                    best = Some((d, true, m.id));
                }
            }
        }
        if self.cfg.pvp {
            for q in self.players.iter().flatten() {
                let t = (q.pos.x, q.pos.z);
                if q.info.id != owner && q.has_pos && q.down_until.is_none() && q.pos.flags & FLAG_FLY == 0 && (q.pos.flags & FLAG_SNAP != 0 || (q.pos.y - ay).abs() < 6.0) && in_arc(a, yaw, t, reach, arc, tr) {
                    let d = ((t.0 - a.0).powi(2) + (t.1 - a.1).powi(2)).sqrt();
                    if best.map_or(true, |b| d < b.0) {
                        best = Some((d, false, q.info.id));
                    }
                }
            }
        }
        let Some((_, is_mob, vid)) = best else { return };
        let dmg = damage(&def, &group, 1.0, (self.tick % 11) as f32 / 10.0);
        let grp = damage_type(&group);
        if is_mob {
            self.hit_mob(owner, vid, dmg, &group);
        } else {
            self.hurt_player(vid, dmg, (owner, false), &group, a);
        }
        let _ = grp;
    }

    /// Per-tick combat: mobs think and fight, due blows land, projectiles fly, the fallen get up.
    fn combat(&mut self, dt: f32) {
        let now = Instant::now();
        // players using the free-fly camera are out of the fight (a jump is not: jumping players can still be hit)
        let living: Vec<(u16, f32, f32)> = self.players.iter().flatten().filter(|p| p.has_pos && p.down_until.is_none() && p.pos.flags & FLAG_FLY == 0).map(|p| (p.info.id, p.pos.x, p.pos.z)).collect();
        let blows: Vec<MobAttack> = self.mobs.step(dt, &living);
        for b in blows {
            let m = &self.mobs.mobs[b.mob as usize];
            let from = (m.pos.x, m.pos.z);
            self.hurt_player(b.player, b.dmg, (b.mob, true), "mob", from);
        }
        // swings that reach their hit moment
        let due: Vec<u16> = self.pending.iter().filter(|p| p.due <= now).map(|p| p.owner).collect();
        self.pending.retain(|p| p.due > now);
        for owner in due {
            self.melee_lands(owner);
        }
        // arrows and bolts
        let mut shots = std::mem::take(&mut self.shots);
        shots.retain_mut(|s| {
            let (a, b) = s.step(dt);
            if s.left <= 0.0 || b[1] < -500.0 {
                return false;
            }
            let r = BODY_RADIUS * WORLD_SCALE + 0.3;
            let mut best: Option<(f32, bool, u16, f32)> = None; // (distance from the start of the segment, is mob, id, ref y)
            let mid = ((a[0] + b[0]) * 0.5, (a[2] + b[2]) * 0.5);
            for i in self.mobs.candidates(mid.0, mid.1) {
                let m = &self.mobs.mobs[i as usize];
                if m.alive && (m.pos.x - mid.0).abs() < 60.0 && (m.pos.z - mid.1).abs() < 60.0 && segment_hits(a, b, (m.pos.x, m.pos.z), r + 0.2, a[1]) {
                    let d = (m.pos.x - a[0]).powi(2) + (m.pos.z - a[2]).powi(2);
                    if best.map_or(true, |x| d < x.0) {
                        best = Some((d, true, m.id, 0.0));
                    }
                }
            }
            if self.cfg.pvp {
                for q in self.players.iter().flatten() {
                    if q.info.id != s.owner && q.has_pos && q.down_until.is_none() && q.pos.flags & FLAG_FLY == 0 && segment_hits(a, b, (q.pos.x, q.pos.z), r, if q.pos.flags & FLAG_SNAP != 0 { a[1] } else { q.pos.y + 1.5 }) {
                        let d = (q.pos.x - a[0]).powi(2) + (q.pos.z - a[2]).powi(2);
                        if best.map_or(true, |x| d < x.0) {
                            best = Some((d, false, q.info.id, 0.0));
                        }
                    }
                }
            }
            let Some((_, is_mob, vid, _)) = best else { return true };
            if is_mob {
                self.hit_mob(s.owner, vid, s.dmg, &s.group);
            } else {
                self.hurt_player(vid, s.dmg, (s.owner, false), &s.group, (a[0], a[2]));
            }
            false
        });
        self.shots.append(&mut shots);
        // bodies rot away
        let gone: Vec<u32> = self.corpses.iter().filter(|c| c.expires <= now).map(|c| c.cid).collect();
        if !gone.is_empty() {
            self.corpses.retain(|c| c.expires > now);
            for cid in gone {
                self.broadcast_text(serde_json::to_string(&ServerMsg::CorpseGone { cid }).unwrap(), None);
            }
        }
        // get up
        let up: Vec<u16> = self.players.iter().flatten().filter(|p| p.down_until.map_or(false, |t| t <= now)).map(|p| p.info.id).collect();
        for id in up {
            self.respawn(id, true);
        }
    }

    fn chat(&mut self, id: u16, text: &str) {
        let text = clean(text, MAX_CHAT);
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        if text.is_empty() || p.last_chat.elapsed() < Duration::from_millis(400) {
            return;
        }
        p.last_chat = Instant::now();
        let msg = serde_json::to_string(&ServerMsg::Chat { id, name: &p.info.name, text: &text }).unwrap();
        self.broadcast_text(msg, None);
    }

    fn remove(&mut self, id: u16) {
        if let Some(slot) = self.players.get_mut(id as usize) {
            if let Some(p) = slot.take() {
                self.online -= 1;
                println!("- {} (id {}) — {} online", p.info.name, id, self.online);
                let msg = serde_json::to_string(&ServerMsg::Left { id }).unwrap();
                self.broadcast_text(msg, None);
            }
        }
    }

    /// Remove players whose channel overflowed or closed (removal broadcasts `Left`, which may in turn overflow someone else).
    fn reap(&mut self) {
        loop {
            let dead: Vec<u16> = self.players.iter().flatten().filter(|p| p.dead).map(|p| p.info.id).collect();
            if dead.is_empty() {
                return;
            }
            for id in dead {
                self.remove(id);
            }
        }
    }

    fn broadcast_text(&mut self, text: String, except: Option<u16>) {
        for p in self.players.iter_mut().flatten() {
            if Some(p.info.id) != except && p.tx.try_send(Message::Text(text.clone())).is_err() {
                p.dead = true;
            }
        }
    }

    /// One network tick: build a per-recipient snapshot of everybody within the interest radius (spatial hash, 3x3 cells).
    fn step(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.combat(1.0 / self.cfg.tick_hz as f32);
        let watchers: Vec<(f32, f32)> = self.players.iter().flatten().filter(|p| p.has_pos).map(|p| (p.pos.x, p.pos.z)).collect();
        self.stats.lod = self.npcs.step(1.0 / self.cfg.tick_hz as f32, &watchers, self.cfg.radius);
        let cell = self.cfg.radius.max(1.0);
        let r2 = self.cfg.radius * self.cfg.radius;
        let ents: Vec<(u16, PosUpdate)> = self.players.iter().flatten().filter(|p| p.has_pos).map(|p| (p.info.id, p.pos)).chain(self.npcs.active().map(|n| (n.id, n.pos))).collect();
        let key = |x: f32, z: f32| ((x / cell).floor() as i32, (z / cell).floor() as i32);
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, (_, p)) in ents.iter().enumerate() {
            grid.entry(key(p.x, p.z)).or_default().push(i as u32);
        }
        let mut buf: Vec<u8> = Vec::with_capacity(7 + 20 * 128);
        for p in self.players.iter_mut().flatten() {
            if !p.has_pos {
                continue;
            }
            buf.clear();
            buf.push(OP_SNAPSHOT);
            buf.extend_from_slice(&self.tick.to_le_bytes());
            buf.extend_from_slice(&[0, 0]);
            let (cx, cz) = key(p.pos.x, p.pos.z);
            let mut count: u16 = 0;
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = grid.get(&(cx + dx, cz + dz)) else { continue };
                    for &i in list {
                        let (id, q) = &ents[i as usize];
                        let (ddx, ddz) = (q.x - p.pos.x, q.z - p.pos.z);
                        if *id == p.info.id || ddx * ddx + ddz * ddz > r2 {
                            continue;
                        }
                        buf.extend_from_slice(&id.to_le_bytes());
                        q.write_body(&mut buf);
                        count += 1;
                    }
                }
            }
            if count > 0 || p.last_sent != 0 {
                p.last_sent = count as usize;
                buf[5..7].copy_from_slice(&count.to_le_bytes());
                match p.tx.try_send(Message::Binary(buf.clone())) {
                    Ok(()) => {
                        self.stats.bytes_out += buf.len() as u64;
                        self.stats.snapshots_out += 1;
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => self.stats.dropped += 1,
                    Err(mpsc::error::TrySendError::Closed(_)) => p.dead = true,
                }
            }
            if self.tick % MOB_EVERY != 0 || self.mobs.mobs.is_empty() {
                continue;
            }
            buf.clear();
            buf.push(OP_MOBS);
            buf.extend_from_slice(&self.tick.to_le_bytes());
            buf.extend_from_slice(&[0, 0]);
            let mut n: u16 = 0;
            for i in self.mobs.candidates(p.pos.x, p.pos.z) {
                let m = &self.mobs.mobs[i as usize];
                let (ddx, ddz) = (m.pos.x - p.pos.x, m.pos.z - p.pos.z);
                if ddx * ddx + ddz * ddz > r2 {
                    continue;
                }
                buf.extend_from_slice(&m.id.to_le_bytes());
                buf.extend_from_slice(&m.kind.to_le_bytes());
                m.pos.write_body(&mut buf);
                n += 1;
            }
            if n == 0 && p.last_mobs == 0 {
                continue;
            }
            p.last_mobs = n as usize;
            buf[5..7].copy_from_slice(&n.to_le_bytes());
            match p.tx.try_send(Message::Binary(buf.clone())) {
                Ok(()) => {
                    self.stats.bytes_out += buf.len() as u64;
                    self.stats.snapshots_out += 1;
                }
                Err(mpsc::error::TrySendError::Full(_)) => self.stats.dropped += 1,
                Err(mpsc::error::TrySendError::Closed(_)) => p.dead = true,
            }
        }
        self.reap();
    }

    /// Write the NPC population to the database (when there is one).
    fn save(&mut self) {
        let Some(store) = self.store.as_mut() else { return };
        if let Err(e) = store.save(self.npcs.places.len(), &self.npcs.states()) {
            eprintln!("npc save failed: {e}");
        }
    }

    fn report(&mut self) {
        let s = std::mem::take(&mut self.stats);
        let avg = if s.ticks > 0 { s.tick_us_sum / s.ticks as u128 } else { 0 };
        println!(
            "[stats] {} online | npcs {} active/{} near/{} far | tick avg {} µs max {} µs | out {:.1} KB/s ({:.0} snapshots/s) | dropped {}",
            self.online, s.lod.active, s.lod.near, s.lod.far, avg, s.tick_us_max, s.bytes_out as f64 / 5.0 / 1024.0, s.snapshots_out as f64 / 5.0, s.dropped
        );
    }
}
