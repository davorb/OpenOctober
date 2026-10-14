//! The world task: one task owns every player, so there are no locks. Connection tasks talk to it through `Cmd`s and receive
//! messages through their own bounded channel (a slow client drops position snapshots, and is kicked if it can't keep up with events).
use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;

use crate::combat::*;
use crate::mobs::{MobAttack, Mobs};
use crate::nav::Nav;
use crate::economy::{item_weight, Stock, CARRY_LIMIT};
use crate::npcs::{chaos_race, Role, Temper, VENDOR_DOWN_SECS, Goal, Lod, LodCounts, Npcs, DOWN_SECS as NPC_DOWN_SECS, NPC_ID_BASE};
use crate::proto::*;
use crate::store::Store;

#[derive(Clone, Debug)]
pub struct Config {
    /// Players further apart than this (metres, x/z plane) don't receive each other's position updates.
    pub radius: f32,
    pub max_players: usize,
    pub tick_hz: u32,
    /// Path of the mob spawn JSON (`game/data/mobspawns.json`); `None` runs without mobs.
    pub spawns: Option<String>,
    /// Path of `game/data/weapons.json` (stats, clip lengths); `None` = combat off (equip / swing messages are still relayed).
    pub weapons: Option<String>,
    /// Players can hurt each other.
    pub pvp: bool,
    /// Number of living NPCs (0 = none) and the data they live from: `game/data/pois.json` and `spawns.json` (the racial capitals).
    pub npcs: usize,
    pub pois: Option<String>,
    pub capitals: Option<String>,
    /// Seed of the NPC population.
    pub npc_seed: u32,
    /// Directory of baked nav tiles (`game/tools/build_nav.js` writes `server/data/nav`); `None` = NPCs ignore terrain and walk straight lines.
    pub nav: Option<String>,
    /// `game/data/items.json` and `vendors.json`: the goods and the shopkeepers (from `tools/build_items.js` / `build_vendors.js`); without both there is no trade.
    pub items: Option<String>,
    pub vendors: Option<String>,
    /// A player can only wield a weapon he owns (it is in his stock, or already in his hand); everybody starts, and gets up, with the worst weapon of the game.
    pub own_weapons: bool,
    /// A body lies there this many seconds, then turns into a lootable tombstone that lasts `tomb_secs`.
    pub body_secs: f32,
    pub tomb_secs: u64,
    /// How long taking everything from a tombstone takes (a channel: damage or walking away interrupts it).
    pub loot_secs: f32,
    /// NPC time runs this many times faster than real time (1.0 = normal; for watching NPC life without waiting).
    pub npc_time_scale: f32,
    /// SQLite file the NPCs are saved to and restored from; `None` = they start fresh every time.
    pub db: Option<String>,
    /// Fastest a player may move on the ground / flying (m/s, x/z plane; 0 = no limit). Clients are authoritative for their position, so a faster
    /// update is cut short at the distance the player could have covered (see `limit_move`). Connections from the local machine are exempt (`Cmd::Trust`).
    pub max_speed: f32,
    pub max_fly_speed: f32,
    /// A player who loses his connection stays standing (killable, lootable once dead) this many seconds.
    pub ghost_secs: f32,
    /// `/logout` takes this many seconds, during which the player must stand still (everybody hears it).
    pub logout_secs: f32,
}

/// A player may save up this many seconds of movement allowance (a stall, a tab in the background) and spend it in one update.
const MOVE_BURST_SECS: f32 = 2.0;

/// Where a player who reports `to` can be when he was at `from` and may move `budget` metres (x/z): the reported spot, or the point `budget` metres towards it.
fn limit_move(from: &PosUpdate, to: PosUpdate, budget: f32) -> (PosUpdate, f32) {
    let (dx, dz) = (to.x - from.x, to.z - from.z);
    let d = (dx * dx + dz * dz).sqrt();
    if d <= budget {
        return (to, d);
    }
    let k = budget / d;
    (PosUpdate { x: from.x + dx * k, y: from.y, z: from.z + dz * k, ..to }, budget)
}

/// Seconds between autosaves of the NPC population.
const SAVE_EVERY: u64 = 30;

/// Mob positions are sent every this many ticks (they walk at ~1 m/s, so clients interpolate).
const MOB_EVERY: u32 = 3;

pub struct JoinReq {
    /// The account this character belongs to (`None`: no account, e.g. a test or a bot): such a player is removed the moment he disconnects.
    pub account: Option<i64>,
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
    /// A player takes everything from the tombstone `cid`.
    Loot(u16, u32),
    /// Put `Stock` into a player's pack. Not reachable from the network (`main.rs` does not map any client message to it): for tests and, later, game masters.
    Give(u16, Stock),
    Shop(u16, u16),
    Buy(u16, u16, String, u32),
    Sell(u16, u16, String, u32),
    /// The connection comes from the local machine (development): exempt from the movement limit.
    Trust(u16),
    /// Put a player where his account says he is (a login, not a client's claim): the movement limit then measures from here.
    Place(u16, PosUpdate),
    /// Where is this player (x, y, z, yaw), and is he dead? `None` before his first position. For saving the character.
    Where(u16, oneshot::Sender<Option<([f32; 4], bool)>>),
}

/// A blow whose animation has started and that lands at `due`.
struct Pending {
    due: Instant,
    owner: u16,
}

/// What `melee_lands` needs to know about whoever swings.
struct Fighter {
    pos: PosUpdate,
    weapon: String,
    group: String,
}

/// A player can trade with a vendor from this close (metres).
const SHOP_REACH: f32 = 8.0;

/// A player can take from a tombstone this close (metres), and it takes this long (seconds), interrupted by damage or by stepping away.
const LOOT_REACH: f32 = 4.0;
const LOOT_STEP_AWAY: f32 = 1.5;


struct Corpse {
    cid: u32,
    /// The body is replaced by the tombstone at `tomb_at`; the tombstone rots at `expires` (set then).
    tomb_at: Instant,
    expires: Option<Instant>,
    /// The JSON the late joiners get: the body's `Corpse` message, later the `Tombstone`.
    msg: String,
    /// What lies on the body, for the tombstone (empty: the body just goes), and who will come back for it.
    stock: Stock,
    owner: Option<u16>,
    info: PlayerInfo,
    pos: PosUpdate,
}

/// A vendor fights mobs this close to it (and no further than GUARD_LEASH from its post).
const GUARD_RADIUS: f32 = 25.0;
const GUARD_LEASH: f32 = 40.0;
/// How long an NPC holds a grudge against a player who hurt it.
const GRUDGE_SECS: f32 = 90.0;
/// A hostile NPC notices a player this close (m); a friendly one greets him from this close.
const NPC_AGGRO: f32 = 28.0;
const NPC_GREET: f32 = 14.0;
const GREETINGS: [&str; 6] = ["Well met, traveller.", "Mind the wolves out there.", "Fine day for a walk.", "Safe roads to you.", "Have you come far?", "Good hunting."];

/// While logging out he may drift this far (metres) from where he began, and move this fast (m/s), before the logout is cancelled.
const LOGOUT_STILL: f32 = 0.35;
const LOGOUT_STILL_SPEED: f32 = 0.3;

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
    account: Option<i64>,
    /// The connection is gone but the body stays until this moment (see `Config::ghost_secs`): anybody can fight it; the owner may log in and take it over.
    ghost_until: Option<Instant>,
    /// `/logout` in progress: when it completes, and where he stood (moving away cancels it).
    logout_until: Option<Instant>,
    logout_at: (f32, f32),
    /// Joined as the fallen body of a ghost that was killed meanwhile: the next `Place` does not count as a speed hack (the client puts him at the capital).
    fresh_death: bool,
    /// What the player carries (gold and goods): picked up from tombstones, dropped on his own when he dies.
    stock: Stock,
    /// A loot channel in progress: tombstone, when it completes, where he stood.
    looting: Option<(u32, Instant, f32, f32)>,
    // combat
    weapon: String,
    group: String,
    drawn: bool,
    blocking: bool,
    hp: f32,
    down_until: Option<Instant>,
    next_attack: Instant,
    // movement limit: metres of allowance left, when it was last topped up, and whether the next update may jump (first one, after a respawn)
    move_budget: f32,
    last_move: Instant,
    free_move: bool,
    trusted: bool,
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
    /// Accounts whose ghost was killed while they were away: they get up at the capital when they log in again.
    fallen: std::collections::HashSet<i64>,
    /// Where each removed player stood last (and whether he was dead): the connection saves it after the player is gone.
    gone: HashMap<u16, ([f32; 4], bool)>,
    /// For the random starting weapon (seeded from the clock: every run draws differently).
    rng: u32,
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
        (n, Some(pois), Some(caps)) if n > 0 => {
            let loaded = match (&cfg.items, &cfg.vendors) {
                (Some(items), Some(vendors)) => Npcs::load_with_economy(pois, caps, items, vendors, n, cfg.npc_seed).or_else(|e| {
                    eprintln!("no trade ({e})");
                    Npcs::load(pois, caps, n, cfg.npc_seed)
                }),
                _ => Npcs::load(pois, caps, n, cfg.npc_seed),
            };
            loaded.unwrap_or_else(|e| {
                eprintln!("no npcs: {e}");
                Npcs::empty()
            })
        }
        _ => Npcs::empty(),
    };
    println!("{} npcs in {} places", npcs.npcs.len(), npcs.places.len());
    let mut npcs = npcs;
    npcs.time_scale = cfg.npc_time_scale.max(0.1);
    npcs.loot_secs = cfg.loot_secs;
    npcs.equip_all(&weapons); // armed first: only an armed NPC thinks of hunting
    npcs.set_grounds(mobs.grounds());
    if let (Some(dir), false) = (&cfg.nav, npcs.npcs.is_empty()) {
        match Nav::open(dir) {
            Ok(nav) => {
                println!("nav: {} tiles", nav.tile_count());
                npcs.nav = Some(nav);
            }
            Err(e) => eprintln!("no nav (npcs walk straight lines): {e}"),
        }
    }
    let store = match (&cfg.db, npcs.npcs.is_empty()) {
        (Some(path), false) => match Store::open(path).and_then(|s| s.load(npcs.places.len(), npcs.vendor_count()).map(|rows| (s, rows))) {
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
    let mut world = World { players: (0..cfg.max_players.min(NPC_ID_BASE as usize)).map(|_| None).collect(), cfg: cfg.clone(), online: 0, cursor: 0, tick: 0, mobs, npcs, store, stats: Stats::default(), weapons, pending: Vec::new(), shots: Vec::new(), corpses: Vec::new(), next_corpse: 1, fallen: Default::default(), gone: HashMap::new(), rng: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ (d.as_secs() as u32)) | 1 };
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
                    let mut p = p;
                    if pl.down_until.is_none() || !pl.has_pos {
                        let now = Instant::now();
                        let speed = if p.flags & FLAG_FLY != 0 { self.cfg.max_fly_speed } else { self.cfg.max_speed };
                        if speed > 0.0 && !pl.trusted && pl.has_pos && !pl.free_move {
                            // metres he may have covered since the last update, with some saved up; the rest of a jump is cut off
                            pl.move_budget = (pl.move_budget + speed * now.duration_since(pl.last_move).as_secs_f32()).min(speed * MOVE_BURST_SECS);
                            let (q, spent) = limit_move(&pl.pos, p, pl.move_budget);
                            pl.move_budget -= spent;
                            p = q;
                        }
                        pl.last_move = now;
                        pl.free_move = false;
                        pl.pos = p;
                        pl.has_pos = true;
                    }
                    // moving cancels a logout
                    if pl.logout_until.is_some() && ((p.x - pl.logout_at.0).powi(2) + (p.z - pl.logout_at.1).powi(2) > LOGOUT_STILL * LOGOUT_STILL || p.speed as f32 > LOGOUT_STILL_SPEED * 10.0 || p.flags & (FLAG_AIR | FLAG_SWIM) != 0) {
                        self.cancel_logout(id);
                    }
                    let Some(Some(pl)) = self.players.get_mut(id as usize) else { return };
                    // walking away interrupts a loot channel
                    if let Some((cid, _, x, z)) = pl.looting {
                        if (p.x - x).powi(2) + (p.z - z).powi(2) > LOOT_STEP_AWAY * LOOT_STEP_AWAY {
                            self.end_loot(id, cid, false);
                        }
                    }
                }
            }
            Cmd::Trust(id) => {
                if let Some(Some(pl)) = self.players.get_mut(id as usize) {
                    pl.trusted = true;
                }
            }
            Cmd::Place(id, p) => {
                if let Some(Some(pl)) = self.players.get_mut(id as usize) {
                    pl.pos = p;
                    pl.has_pos = true;
                    pl.free_move = std::mem::take(&mut pl.fresh_death);
                    pl.move_budget = 0.0;
                    pl.last_move = Instant::now();
                }
            }
            Cmd::Where(id, reply) => {
                let at = self.players.get(id as usize).and_then(|p| p.as_ref()).filter(|p| p.has_pos).map(|p| ([p.pos.x, p.pos.y, p.pos.z, p.pos.yaw], p.down_until.is_some())).or_else(|| self.gone.get(&id).copied());
                let _ = reply.send(at);
            }
            Cmd::Chat(id, text) => self.chat(id, &text),
            Cmd::Leave(id) => self.disconnect(id),
            Cmd::Equip(id, w, drawn) if !self.logging_out(id) => self.equip(id, w, drawn),
            Cmd::Attack(id, kind, clip) if !self.logging_out(id) => self.attack(id, &kind, &clip),
            Cmd::Block(id, on) if !self.logging_out(id) => self.block(id, on),
            Cmd::Bow(id, what) if !self.logging_out(id) => self.relay(id, |id| serde_json::to_string(&ServerMsg::Bow { id, what: &what }).unwrap()),
            Cmd::Shoot(id, from, dir, power, spell) if !self.logging_out(id) => self.shoot(id, from, dir, power, spell),
            Cmd::Respawn(id) => self.respawn(id, false),
            Cmd::Loot(id, cid) if !self.logging_out(id) => self.loot_grave(id, cid),
            Cmd::Give(id, stock) => {
                if let Some(Some(p)) = self.players.get_mut(id as usize) {
                    p.stock.gold += stock.gold;
                    for (k, q) in stock.items {
                        p.stock.add(&k, q);
                    }
                }
                self.send_stock(id);
            }
            Cmd::Shop(id, npc) if !self.logging_out(id) => self.open_shop(id, npc),
            Cmd::Buy(id, npc, item, qty) if !self.logging_out(id) => self.trade(id, npc, &item, qty, true),
            Cmd::Sell(id, npc, item, qty) if !self.logging_out(id) => self.trade(id, npc, &item, qty, false),
            // somebody who is logging out does nothing but wait (moving away cancels it)
            Cmd::Equip(..) | Cmd::Attack(..) | Cmd::Block(..) | Cmd::Bow(..) | Cmd::Shoot(..) | Cmd::Loot(..) | Cmd::Shop(..) | Cmd::Buy(..) | Cmd::Sell(..) => {}
        }
        self.reap();
    }

    fn join(&mut self, req: JoinReq) {
        if !RACES.contains(&req.race.as_str()) || !GENDERS.contains(&req.gender.as_str()) {
            let _ = req.reply.send(Err("unknown race or gender"));
            return;
        }
        if let Some(acc) = req.account {
            // his body is still standing from an earlier connection: he takes it over
            if let Some(id) = self.players.iter().position(|p| p.as_ref().is_some_and(|p| p.account == Some(acc) && p.ghost_until.is_some())) {
                let p = self.players[id].as_mut().unwrap();
                p.ghost_until = None;
                p.tx = req.tx;
                p.last_sent = 0;
                p.last_mobs = 0;
                let _ = req.reply.send(Ok(id as u16));
                self.welcome(id);
                self.send_stock(id as u16);
                println!("+ {} (id {}) took over his body — {} online", self.players[id].as_ref().unwrap().info.name, id, self.online);
                return;
            }
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
        let info = PlayerInfo { id: id as u16, name, race: req.race, gender: req.gender, gear: clean(&req.gear, MAX_GEAR), weapon: String::new(), drawn: false, npc: false, role: String::new() };
        let fallen = req.account.is_some_and(|a| self.fallen.remove(&a));
        self.gone.remove(&(id as u16));
        self.players[id] = Some(Player { info, pos: PosUpdate::default(), has_pos: false, tx: req.tx, last_chat: Instant::now() - Duration::from_secs(10), last_sent: 0, last_mobs: 0, dead: false, account: req.account, ghost_until: None, logout_until: None, logout_at: (0.0, 0.0), fresh_death: fallen, stock: Stock::default(), looting: None, weapon: String::new(), group: String::new(), drawn: false, blocking: false, hp: PLAYER_HP, down_until: None, next_attack: Instant::now(), move_budget: 0.0, last_move: Instant::now(), free_move: true, trusted: false });
        self.online += 1;
        let _ = req.reply.send(Ok(id as u16));
        if !self.welcome(id) {
            return;
        }
        self.give_starter_weapon(id as u16);
        self.send_stock(id as u16);
        let me = self.players[id].as_ref().unwrap();
        let joined = ServerMsg::Joined { player: &me.info };
        let text = serde_json::to_string(&joined).unwrap();
        self.broadcast_text(text, Some(id as u16));
        if fallen {
            // his body was killed while he was away: he gets up at the capital like anybody who died
            self.send_to(id as u16, serde_json::to_string(&ServerMsg::Respawn { id: id as u16, hp: PLAYER_HP, max: PLAYER_HP }).unwrap());
        }
        println!("+ {} ({}_{}) id {} — {} online", self.players[id].as_ref().unwrap().info.name, self.players[id].as_ref().unwrap().info.race, self.players[id].as_ref().unwrap().info.gender, id, self.online);
    }

    /// What a connection gets on entering: the roster, the mob types, the bodies on the ground and the logouts in progress. false = it could not take it.
    fn welcome(&mut self, id: usize) -> bool {
        let me = self.players[id].as_ref().unwrap();
        let all: Vec<&PlayerInfo> = self.players.iter().flatten().map(|p| &p.info).chain(self.npcs.infos()).collect();
        let mut msgs = vec![serde_json::to_string(&ServerMsg::Welcome { id: id as u16, radius: self.cfg.radius, tick_hz: self.cfg.tick_hz, players: all }).unwrap()];
        if me.ghost_until.is_none() && !me.info.weapon.is_empty() && me.has_pos {
            // a body taken over: tell the client what is in its hand and how it is
            msgs.push(serde_json::to_string(&ServerMsg::Equip { id: id as u16, w: &me.info.weapon, drawn: me.info.drawn }).unwrap());
            msgs.push(serde_json::to_string(&ServerMsg::Hp { hp: me.hp, max: PLAYER_HP }).unwrap());
        }
        if !self.mobs.types.is_empty() {
            msgs.push(serde_json::to_string(&ServerMsg::Mobtypes { types: &self.mobs.types }).unwrap());
        }
        msgs.extend(self.corpses.iter().map(|c| c.msg.clone()));
        let now = Instant::now();
        for p in self.players.iter().flatten().filter(|p| p.info.id as usize != id) {
            if let Some(t) = p.logout_until {
                msgs.push(serde_json::to_string(&ServerMsg::Logout { id: p.info.id, secs: t.saturating_duration_since(now).as_secs_f32() }).unwrap());
            }
        }
        let p = self.players[id].as_mut().unwrap();
        for m in msgs {
            if p.tx.try_send(Message::Text(m)).is_err() {
                p.dead = true;
                return false;
            }
        }
        true
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
        let own = self.owns_weapons();
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        let mut stock_changed = false;
        if own && p.weapon != w {
            // wielding a weapon takes it out of the pack (the one in the hand goes back in)
            if !w.is_empty() && p.stock.count(&w) == 0 {
                let (cur, drawn) = (p.weapon.clone(), p.drawn);
                let msg = serde_json::to_string(&ServerMsg::Equip { id, w: &cur, drawn }).unwrap();
                let _ = p.tx.try_send(Message::Text(msg)); // refused: he does not have it; tell him what he holds
                return;
            }
            p.stock.take(&w, 1);
            if !p.weapon.is_empty() {
                p.stock.add(&std::mem::take(&mut p.weapon), 1);
            }
            stock_changed = true;
        }
        p.weapon = w.clone();
        p.group = group;
        p.drawn = drawn && !w.is_empty();
        p.blocking = false;
        p.info.weapon = w.clone();
        p.info.drawn = p.drawn;
        let drawn = p.drawn;
        self.relay(id, |id| serde_json::to_string(&ServerMsg::Equip { id, w: &w, drawn }).unwrap());
        if stock_changed {
            self.send_stock(id);
        }
    }

    /// Ownership of weapons is enforced when it is switched on and the game has a weapon table (without one nobody could ever hold a weapon).
    fn owns_weapons(&self) -> bool {
        self.cfg.own_weapons && !self.weapons.defs.is_empty()
    }

    /// A new character, and one that has got up again, holds the worst weapon of the game (sheathed): he told about it as well as everybody near.
    fn give_starter_weapon(&mut self, id: u16) {
        if !self.owns_weapons() {
            return;
        }
        // one of the weapons that tie for the worst, at random: a new start, or a getting up, is a new draw
        let set: Vec<String> = self.weapons.worst_melee_set().into_iter().map(str::to_string).collect();
        if set.is_empty() {
            return;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let worst = set[self.rng as usize % set.len()].clone();
        let Some(group) = self.weapons.get(&worst).and_then(|d| d.group.clone()) else { return };
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        if !p.weapon.is_empty() {
            return;
        }
        p.weapon = worst.clone();
        p.group = group;
        p.drawn = false;
        p.info.weapon = worst.clone();
        p.info.drawn = false;
        let msg = serde_json::to_string(&ServerMsg::Equip { id, w: &worst, drawn: false }).unwrap();
        let _ = p.tx.try_send(Message::Text(msg.clone()));
        self.relay(id, |id| serde_json::to_string(&ServerMsg::Equip { id, w: &worst, drawn: false }).unwrap());
        self.send_stock(id);
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
        if !power.is_finite() || !from.iter().chain(dir.iter()).all(|v| v.is_finite() && v.abs() < 1.0e6) {
            return; // NaN / infinity would poison the projectile (a never-ending one, or damage of NaN)
        }
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        let near = (from[0] - p.pos.x).powi(2) + (from[2] - p.pos.z).powi(2) < 100.0 && (from[1] - p.pos.y).abs() < 100.0;
        if p.down_until.is_some() || !p.drawn || !ok_group || !(len >= 0.5) || !near {
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
        let Some(def) = self.weapons.get(&p.weapon).cloned().or_else(|| self.weapons.defs.is_empty().then(|| WeaponDef { group: Some(group.clone()), kind: String::new(), reach: None, angle: None, rank: None, natural: false, src: None })) else { return };
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
        p.free_move = true; // the client puts him at a spawn point: not a speed hack
        p.hp = PLAYER_HP;
        let msg = serde_json::to_string(&ServerMsg::Respawn { id, hp: PLAYER_HP, max: PLAYER_HP }).unwrap();
        self.broadcast_text(msg, None);
        self.give_starter_weapon(id); // he dropped what he held: he gets up with the worst weapon of the game
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
        let interrupted = p.looting.map(|l| l.0);
        let (at, hp, snap) = ([p.pos.x, p.pos.y, p.pos.z], p.hp, p.pos.flags & FLAG_SNAP != 0);
        if let Some(cid) = interrupted {
            self.end_loot(vid, cid, false); // damage interrupts a loot channel
        }
        let msg = serde_json::to_string(&ServerMsg::Hit { a: by.0, am: by.1, v: "p", i: vid, dmg, hp, max: PLAYER_HP, grp, blocked, at, snap, killed }).unwrap();
        self.send_near(at[0], at[2], msg, None);
        if killed {
            self.send_near(at[0], at[2], serde_json::to_string(&ServerMsg::Died { v: "p", i: vid }).unwrap(), None);
            self.cancel_logout(vid);
            self.leave_corpse(vid);
            // a body that stood there without its owner is gone now: he gets up at the capital when he comes back
            if let Some(Some(p)) = self.players.get(vid as usize) {
                if let (Some(acc), true) = (p.account, p.ghost_until.is_some()) {
                    self.fallen.insert(acc);
                    self.remove(vid);
                }
            }
        }
    }

    /// Is this player in the middle of a `/logout`? Then he can do nothing but wait (or move, which cancels it).
    fn logging_out(&self, id: u16) -> bool {
        self.players.get(id as usize).and_then(|p| p.as_ref()).is_some_and(|p| p.logout_until.is_some())
    }

    /// `/logout`: for `logout_secs` everybody hears it; he has to stand still, and can use nothing in the meantime.
    fn start_logout(&mut self, id: u16) {
        let secs = self.cfg.logout_secs;
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        if p.logout_until.is_some() || p.ghost_until.is_some() || p.down_until.is_some() || !p.has_pos || p.pos.flags & FLAG_FLY != 0 {
            return;
        }
        p.logout_until = Some(Instant::now() + Duration::from_secs_f32(secs));
        p.logout_at = (p.pos.x, p.pos.z);
        p.blocking = false;
        let interrupted = p.looting.map(|l| l.0);
        if let Some(cid) = interrupted {
            self.end_loot(id, cid, false);
        }
        self.broadcast_text(serde_json::to_string(&ServerMsg::Logout { id, secs }).unwrap(), None);
    }

    fn cancel_logout(&mut self, id: u16) {
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        if p.logout_until.take().is_some() {
            self.broadcast_text(serde_json::to_string(&ServerMsg::LogoutCancel { id }).unwrap(), None);
        }
    }

    /// The connection of `id` is gone. A player in the world leaves his body standing for `ghost_secs`; anybody can still fight it, and it is lootable once dead.
    /// Someone who was dead, flying, or never got into the world is just removed.
    fn disconnect(&mut self, id: u16) {
        let Some(Some(p)) = self.players.get(id as usize) else { return };
        if p.account.is_none() || !p.has_pos || p.down_until.is_some() || p.pos.flags & FLAG_FLY != 0 || p.ghost_until.is_some() || p.dead {
            self.remove(id);
            return;
        }
        self.cancel_logout(id);
        let interrupted = self.players[id as usize].as_ref().and_then(|p| p.looting.map(|l| l.0));
        let p = self.players[id as usize].as_mut().unwrap();
        // nobody is listening any more: whatever is sent to him goes nowhere (and must not count as a dead connection)
        let (tx, mut rx) = mpsc::channel::<Message>(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        p.tx = tx;
        p.blocking = false;
        p.ghost_until = Some(Instant::now() + Duration::from_secs_f32(self.cfg.ghost_secs));
        if let Some(cid) = interrupted {
            self.end_loot(id, cid, false);
        }
        println!("~ {} (id {}) lost his connection: the body stays {} s", self.players[id as usize].as_ref().unwrap().info.name, id, self.cfg.ghost_secs);
    }

    /// Logouts that have run their course, and bodies nobody has come back to.
    fn age_players(&mut self, now: Instant) {
        let done: Vec<u16> = self.players.iter().flatten().filter(|p| p.logout_until.is_some_and(|t| t <= now) || p.ghost_until.is_some_and(|t| t <= now)).map(|p| p.info.id).collect();
        for id in done {
            if self.logging_out(id) {
                self.send_to(id, serde_json::to_string(&ServerMsg::LoggedOut).unwrap());
            }
            self.remove(id);
        }
    }

    /// The body of a player who just died stays for CORPSE_SECS, visible to everybody.
    fn leave_corpse(&mut self, id: u16) {
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        let (info, pos) = (p.info.clone(), p.pos);
        let mut stock = std::mem::take(&mut p.stock); // everything he carried lies on the body ...
        if !p.weapon.is_empty() {
            stock.add(&std::mem::take(&mut p.weapon), 1); // ... the weapon from his hand too
            p.group.clear();
            p.drawn = false;
            p.info.weapon.clear();
            p.info.drawn = false;
            self.relay(id, |id| serde_json::to_string(&ServerMsg::Equip { id, w: "", drawn: false }).unwrap());
        }
        self.send_stock(id);
        self.add_corpse(&info, pos, stock, None);
    }

    /// A body falls: everybody sees it for `body_secs`. Then it turns into a tombstone with `stock` on it (or, empty-handed, just goes). `owner`: an
    /// NPC that will come back for its things.
    fn add_corpse(&mut self, info: &PlayerInfo, pos: PosUpdate, stock: Stock, owner: Option<u16>) {
        let cid = self.next_corpse;
        self.next_corpse += 1;
        let msg = serde_json::to_string(&ServerMsg::Corpse { cid, name: &info.name, race: &info.race, gender: &info.gender, gear: &info.gear, x: pos.x, y: pos.y, z: pos.z, yaw: pos.yaw, snap: pos.flags & FLAG_SNAP != 0 }).unwrap();
        let tomb_at = Instant::now() + Duration::from_secs_f32(self.cfg.body_secs);
        self.corpses.push(Corpse { cid, tomb_at, expires: None, msg: msg.clone(), stock, owner, info: info.clone(), pos });
        self.broadcast_text(msg, None);
    }

    /// Bodies that have lain long enough become tombstones; tombstones rot after `tomb_secs`.
    fn age_corpses(&mut self, now: Instant) {
        let mut turned: Vec<(u32, String)> = Vec::new();
        let mut gone: Vec<u32> = Vec::new();
        let tomb_secs = Duration::from_secs(self.cfg.tomb_secs);
        for c in self.corpses.iter_mut() {
            match c.expires {
                None if c.tomb_at <= now => {
                    let stock = std::mem::take(&mut c.stock);
                    if stock.gold <= 0.0 && stock.items.is_empty() {
                        gone.push(c.cid); // nothing to leave
                        continue;
                    }
                    c.expires = Some(now + tomb_secs);
                    let (gold, units) = (stock.gold, stock.units());
                    c.msg = serde_json::to_string(&ServerMsg::Tombstone { cid: c.cid, name: &c.info.name, x: c.pos.x, y: c.pos.y, z: c.pos.z, yaw: c.pos.yaw, snap: c.pos.flags & FLAG_SNAP != 0, gold, units }).unwrap();
                    turned.push((c.cid, c.msg.clone()));
                    self.npcs.add_grave(c.cid, c.pos.x, c.pos.z, stock, c.owner);
                }
                Some(t) if t <= now => gone.push(c.cid),
                _ => {}
            }
        }
        for (_, msg) in turned {
            self.broadcast_text(msg, None);
        }
        if !gone.is_empty() {
            self.corpses.retain(|c| !gone.contains(&c.cid));
            for cid in gone {
                self.npcs.remove_grave(cid);
                self.broadcast_text(serde_json::to_string(&ServerMsg::CorpseGone { cid }).unwrap(), None);
            }
        }
    }

    /// Tell player `id` what he carries.
    fn send_stock(&mut self, id: u16) {
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        let eco = self.npcs.economy();
        let weight = p.stock.weight(&eco.items) + if p.weapon.is_empty() { 0.0 } else { crate::economy::item_weight(&eco.items, &p.weapon) };
        drop(eco);
        let msg = serde_json::to_string(&ServerMsg::Stock { gold: p.stock.gold, items: &p.stock.items, weight, limit: CARRY_LIMIT, own: self.cfg.own_weapons && !self.weapons.defs.is_empty() }).unwrap();
        if p.tx.try_send(Message::Text(msg)).is_err() {
            p.dead = true;
        }
    }

    /// A player starts taking what lies on the tombstone `cid`: he has to be up and within a few metres, and then to stand still for `LOOT_SECS`.
    fn loot_grave(&mut self, id: u16, cid: u32) {
        let Some(Some(p)) = self.players.get(id as usize) else { return };
        if p.down_until.is_some() || !p.has_pos {
            return;
        }
        let Some(g) = self.npcs.grave(cid) else { return };
        if (g.x - p.pos.x).powi(2) + (g.z - p.pos.z).powi(2) > LOOT_REACH * LOOT_REACH {
            return;
        }
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        if p.looting.map(|l| l.0) == Some(cid) {
            return; // already at it
        }
        let secs = self.cfg.loot_secs;
        p.looting = Some((cid, Instant::now() + Duration::from_secs_f32(secs), p.pos.x, p.pos.z));
        let msg = serde_json::to_string(&ServerMsg::Looting { cid, secs }).unwrap();
        if p.tx.try_send(Message::Text(msg)).is_err() {
            p.dead = true;
        }
    }

    /// The channel of player `id` ends (interrupted, or `ok`).
    fn end_loot(&mut self, id: u16, cid: u32, ok: bool) {
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        p.looting = None;
        let msg = serde_json::to_string(&ServerMsg::LootEnd { cid, ok }).unwrap();
        if p.tx.try_send(Message::Text(msg)).is_err() {
            p.dead = true;
        }
    }

    /// Loot channels that have run their time: whoever is still standing there takes the contents (if nobody was quicker).
    fn finish_loots(&mut self, now: Instant) {
        let due: Vec<(u16, u32)> = self.players.iter().flatten().filter_map(|p| p.looting.filter(|l| l.1 <= now).map(|l| (p.info.id, l.0))).collect();
        for (id, cid) in due {
            // only what fits in the pack comes along; the rest stays on the tombstone
            let free = {
                let eco = self.npcs.economy();
                self.players.get(id as usize).and_then(|p| p.as_ref()).map_or(0.0, |p| (CARRY_LIMIT - p.stock.weight(&eco.items) - if p.weapon.is_empty() { 0.0 } else { item_weight(&eco.items, &p.weapon) }).max(0.0))
            };
            let taken = { let eco = self.npcs.economy(); self.npcs.take_grave_fitting(cid, free, &eco.items) };
            let ok = taken.is_some();
            if let (Some(taken), Some(Some(p))) = (taken, self.players.get_mut(id as usize)) {
                p.stock.gold += taken.gold;
                for (k, q) in taken.items {
                    p.stock.add(&k, q);
                }
            }
            self.end_loot(id, cid, ok);
            if ok {
                self.send_stock(id);
            }
        }
    }

    /// The vendor index of NPC `npc` if player `id` can trade with it right now, else why not.
    fn vendor_of(&self, id: u16, npc: u16) -> Result<usize, &'static str> {
        let Some(Some(p)) = self.players.get(id as usize) else { return Err("no such vendor") };
        let n = self.npcs.by_id(npc).ok_or("no such vendor")?;
        let v = n.vendor.ok_or("no such vendor")?;
        if p.down_until.is_some() || !p.has_pos || (n.pos.x - p.pos.x).powi(2) + (n.pos.z - p.pos.z).powi(2) > SHOP_REACH * SHOP_REACH {
            return Err("too far");
        }
        if n.down > 0.0 || self.npcs.economy().vendors[v].closed {
            return Err("shut");
        }
        Ok(v)
    }

    fn send_to(&mut self, id: u16, msg: String) {
        if let Some(Some(p)) = self.players.get_mut(id as usize) {
            if p.tx.try_send(Message::Text(msg)).is_err() {
                p.dead = true;
            }
        }
    }

    fn send_trade(&mut self, id: u16, op: &str, item: &str, units: u32, gold: f32, reason: &str) {
        let msg = serde_json::to_string(&ServerMsg::Trade { ok: reason.is_empty(), op, item, units, gold, reason }).unwrap();
        self.send_to(id, msg);
    }

    /// Player `id` opens the shop of the vendor `npc`: he is sent what it sells and what it would pay for what he carries.
    fn open_shop(&mut self, id: u16, npc: u16) {
        match self.vendor_of(id, npc) {
            Ok(v) => self.send_shop(id, npc, v),
            Err(reason) => self.send_trade(id, "shop", "", 0, 0.0, reason),
        }
    }

    fn send_shop(&mut self, id: u16, npc: u16, v: usize) {
        let msg = {
            let eco = self.npcs.economy();
            let ven = &eco.vendors[v];
            let goods: Vec<ShopGood> = ven.stock.items.iter().filter_map(|(k, &q)| ven.ask(&eco.items, k).map(|ask| ShopGood { item: k.clone(), qty: q, ask, w: item_weight(&eco.items, k) })).collect();
            let mine = self.players.get(id as usize).and_then(|p| p.as_ref()).map(|p| p.stock.items.clone()).unwrap_or_default();
            let buys: Vec<ShopBid> = mine.iter().filter_map(|(k, &q)| ven.bid(&eco.items, k).map(|bid| ShopBid { item: k.clone(), qty: q, bid })).collect();
            serde_json::to_string(&ServerMsg::Shop { npc, name: &ven.name, kind: ven.kind.title(), open: !ven.closed, gold: ven.stock.gold, goods, buys }).unwrap()
        };
        self.send_to(id, msg);
    }

    /// `Buy` / `Sell`: up to `qty` units of `item` against vendor `npc`, unit by unit at the price the vendor then has. What a player can carry limits a purchase.
    fn trade(&mut self, id: u16, npc: u16, item: &str, qty: u32, buy: bool) {
        use crate::economy::{Economy, Refusal};
        let op = if buy { "buy" } else { "sell" };
        let v = match self.vendor_of(id, npc) {
            Ok(v) => v,
            Err(reason) => {
                self.send_trade(id, op, item, 0, 0.0, reason);
                return;
            }
        };
        let qty = qty.clamp(1, 1000);
        let outcome: Result<(u32, f32), &'static str> = {
            let mut eco = self.npcs.economy_mut();
            let Economy { items, vendors } = &mut *eco;
            let ven = &mut vendors[v];
            let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
            if items.get(item).is_none() {
                Err("unknown item")
            } else if buy {
                let wielded = if p.weapon.is_empty() { 0.0 } else { item_weight(items, &p.weapon) };
                let free = CARRY_LIMIT - p.stock.weight(items) - wielded;
                let w = item_weight(items, item);
                let fits = if w <= 0.0 { qty } else { (free / w).floor().max(0.0) as u32 };
                if fits == 0 {
                    Err("too heavy")
                } else {
                    match ven.sell_to(items, &mut p.stock, item, qty.min(fits)) {
                        Ok(d) => Ok((d.units, d.gold)),
                        Err(Refusal::UnknownItem) => Err("unknown item"),
                        Err(Refusal::NotInTrade) => Err("not in trade"),
                        Err(Refusal::NothingDone) => Err(if ven.stock.count(item) == 0 { "none left" } else { "too poor" }),
                    }
                }
            } else if p.stock.count(item) == 0 {
                Err("nothing to sell")
            } else {
                match ven.buy_from(items, &mut p.stock, item, qty) {
                    Ok(d) => Ok((d.units, d.gold)),
                    Err(Refusal::UnknownItem) => Err("unknown item"),
                    Err(Refusal::NotInTrade) => Err("not in trade"),
                    Err(Refusal::NothingDone) => Err("vendor cannot pay"),
                }
            }
        };
        match outcome {
            Ok((units, gold)) => {
                self.send_trade(id, op, item, units, gold, "");
                self.send_stock(id);
                self.send_shop(id, npc, v);
            }
            Err(reason) => self.send_trade(id, op, item, 0, 0.0, reason),
        }
    }

    /// A player killed a mob of `rank`: what it drops goes into his pack, as much as fits.
    fn reward_player(&mut self, id: u16, rank: u8, mid: u16) {
        let (gold, drops, free) = {
            let eco = self.npcs.economy();
            let mut rng = (self.tick.wrapping_mul(2654435761) ^ (mid as u32).wrapping_mul(40503) ^ (id as u32 + 1)) | 1;
            let (gold, drops) = crate::economy::loot(&eco.items, rank, &mut rng);
            let free = self.players.get(id as usize).and_then(|p| p.as_ref()).map_or(0.0, |p| (CARRY_LIMIT - p.stock.weight(&eco.items) - if p.weapon.is_empty() { 0.0 } else { item_weight(&eco.items, &p.weapon) }).max(0.0));
            (gold, drops, free)
        };
        let mut free = free;
        let mut got: Vec<(String, u32)> = Vec::new();
        {
            let eco = self.npcs.economy();
            for (k, q) in drops {
                let w = item_weight(&eco.items, &k);
                let n = if w <= 0.0 { q } else { q.min((free / w).floor() as u32) };
                if n > 0 {
                    free -= w * n as f32;
                    got.push((k, n));
                }
            }
        }
        let Some(Some(p)) = self.players.get_mut(id as usize) else { return };
        p.stock.gold += gold;
        for (k, n) in &got {
            p.stock.add(k, *n);
        }
        let msg = serde_json::to_string(&ServerMsg::Drop { gold, items: got }).unwrap();
        self.send_to(id, msg);
        self.send_stock(id);
    }

    /// Everybody is told a tombstone has been emptied: it is gone.
    fn announce_looted(&mut self) {
        let looted: Vec<u32> = std::mem::take(&mut *self.npcs.looted.borrow_mut());
        for cid in looted {
            self.corpses.retain(|c| c.cid != cid);
            self.broadcast_text(serde_json::to_string(&ServerMsg::Looted { cid }).unwrap(), None);
        }
        // a player whose tombstone was emptied by somebody else stops looting it
        let stale: Vec<(u16, u32)> = self.players.iter().flatten().filter_map(|p| p.looting.map(|l| (p.info.id, l.0))).filter(|(_, cid)| self.npcs.grave(*cid).is_none()).collect();
        for (id, cid) in stale {
            self.end_loot(id, cid, false);
        }
    }

    fn hit_mob(&mut self, owner: u16, mid: u16, dmg: f32, grp: &str) {
        let Some((hp, killed)) = self.mobs.damage(mid, dmg, owner) else { return };
        if owner >= NPC_ID_BASE {
            let rank = self.mobs.mobs[mid as usize].rank;
            self.npcs.train_hit(owner, rank); // every blow that lands trains the weapon skill
            if hp <= 0.0 {
                self.npcs.loot_kill(owner, rank); // and a kill leaves loot
            }
        } else if hp <= 0.0 {
            let rank = self.mobs.mobs[mid as usize].rank;
            self.reward_player(owner, rank, mid); // a player's kill drops straight into his pack
        }
        let m = &self.mobs.mobs[mid as usize];
        let (at, max) = ([m.pos.x, 0.0, m.pos.z], m.max_hp);
        let msg = serde_json::to_string(&ServerMsg::Hit { a: owner, am: false, v: "m", i: mid, dmg, hp, max, grp, blocked: false, at, snap: true, killed }).unwrap();
        self.send_near(at[0], at[2], msg, None);
        if killed {
            self.send_near(at[0], at[2], serde_json::to_string(&ServerMsg::Died { v: "m", i: mid }).unwrap(), None);
        }
    }

    /// Whoever swings: a connected player or an NPC (only a drawn weapon in a live, visible fighter counts).
    fn fighter(&self, id: u16) -> Option<Fighter> {
        if id >= NPC_ID_BASE {
            let n = self.npcs.by_id(id)?;
            if n.down > 0.0 || !n.info.drawn || n.lod != Lod::Active {
                return None;
            }
            return Some(Fighter { pos: n.pos, weapon: n.info.weapon.clone(), group: n.group.clone() });
        }
        let Some(Some(p)) = self.players.get(id as usize) else { return None };
        if p.down_until.is_some() || !p.drawn || p.pos.flags & FLAG_FLY != 0 {
            return None;
        }
        Some(Fighter { pos: p.pos, weapon: p.weapon.clone(), group: p.group.clone() })
    }

    /// Apply a blow to NPC `vid` (a mob's or a player's): hit points, death, messages. A dead NPC lies there for `NPC_DOWN_SECS`, then it is back at home.
    fn hurt_npc(&mut self, vid: u16, dmg: f32, by: (u16, bool), grp: &str) {
        let Some(i) = vid.checked_sub(NPC_ID_BASE) else { return };
        let Some(n) = self.npcs.npcs.get_mut(i as usize) else { return };
        if n.down > 0.0 || n.lod != Lod::Active {
            return;
        }
        n.hp = (n.hp - dmg).max(0.0);
        let killed = n.hp <= 0.0;
        if by.0 < NPC_ID_BASE && !by.1 {
            // a player hurt it: it remembers, and turns on him
            n.grudges.retain(|g| g.0 != by.0);
            n.grudges.push((by.0, GRUDGE_SECS));
            if n.foe.is_none() && !killed {
                n.foe = Some(by.0);
            }
        }
        if killed {
            n.down = if n.role == Role::Vendor { VENDOR_DOWN_SECS } else { NPC_DOWN_SECS };
            n.foe = None;
            n.target = None;
            n.chase = None;
            n.info.drawn = false;
            n.pos.speed = 0;
        }
        let (at, hp, max, snap) = ([n.pos.x, n.pos.y, n.pos.z], n.hp, n.max_hp, n.pos.flags & FLAG_SNAP != 0);
        let (info, pos) = (n.info.clone(), n.pos);
        let msg = serde_json::to_string(&ServerMsg::Hit { a: by.0, am: by.1, v: "p", i: vid, dmg, hp, max, grp, blocked: false, at, snap, killed }).unwrap();
        self.send_near(at[0], at[2], msg, None);
        if killed {
            self.send_near(at[0], at[2], serde_json::to_string(&ServerMsg::Died { v: "p", i: vid }).unwrap(), None);
            let dropped = self.npcs.drop_all(vid); // everything it carried, its weapon too
            self.add_corpse(&info, pos, dropped, Some(vid));
        } else {
            self.npcs.train_hurt(vid); // taking blows makes an NPC tougher
        }
    }

    /// Weapons out while an NPC fights, and the fight itself: pick a mob, walk up to it (the NPC's own step does that, from `chase`), strike.
    fn npc_fights(&mut self, dt: f32) {
        let now = Instant::now();
        // stance: drawn while fighting. Everybody who can see it is told.
        let mut stance: Vec<(u16, String, bool, PosUpdate)> = Vec::new();
        for n in self.npcs.npcs.iter_mut() {
            let should = n.lod == Lod::Active && n.down <= 0.0 && (n.foe.is_some() || n.target.is_some() || matches!(n.goal, Goal::Hunt { fighting: true, .. }));
            if n.info.drawn != should && !n.info.weapon.is_empty() {
                n.info.drawn = should;
                if n.lod == Lod::Active {
                    stance.push((n.id, n.info.weapon.clone(), should, n.pos));
                }
            }
        }
        for (id, w, drawn, pos) in stance {
            self.send_near(pos.x, pos.z, serde_json::to_string(&ServerMsg::Equip { id, w: &w, drawn }).unwrap(), None);
        }
        self.npc_social(dt, now);
        self.vendor_guard(dt, now);
        let fighting: Vec<usize> = self.npcs.npcs.iter().enumerate().filter(|(_, n)| n.lod == Lod::Active && n.down <= 0.0 && n.foe.is_none() && matches!(n.goal, Goal::Hunt { fighting: true, .. })).map(|(i, _)| i).collect();
        let mut done: Vec<usize> = Vec::new();
        for i in fighting {
            let n = &self.npcs.npcs[i];
            let Some(def) = self.weapons.get(&n.info.weapon).cloned() else {
                done.push(i);
                continue;
            };
            let (reach, tr) = (def.reach.unwrap_or(2.1) * WORLD_SCALE, BODY_RADIUS * WORLD_SCALE);
            let (p, cap, group, id) = (n.pos, n.hunt_cap(), n.group.clone(), n.id);
            let dist_to = |m: &crate::mobs::Mob| ((m.pos.x - p.x).powi(2) + (m.pos.z - p.z).powi(2)).sqrt();
            let keep = n.target.and_then(|t| self.mobs.mobs.get(t as usize)).filter(|m| m.alive && dist_to(m) < 60.0).map(|m| m.id);
            let target = keep.or_else(|| {
                self.mobs
                    .candidates(p.x, p.z)
                    .map(|k| &self.mobs.mobs[k as usize])
                    .filter(|m| m.alive && m.rank <= cap && dist_to(m) < 45.0)
                    .min_by(|a, b| dist_to(a).total_cmp(&dist_to(b)))
                    .map(|m| m.id)
            });
            let n = &mut self.npcs.npcs[i];
            n.next_attack = (n.next_attack - dt).max(0.0);
            if n.hp < n.max_hp * 0.3 {
                done.push(i); // hurt: leave the fight
                continue;
            }
            let Some(t) = target else {
                n.target = None;
                n.chase = None;
                n.no_target += dt;
                if n.no_target > 8.0 {
                    done.push(i);
                }
                continue;
            };
            let m = &self.mobs.mobs[t as usize];
            let (mx, mz, d) = (m.pos.x, m.pos.z, dist_to(m));
            n.target = Some(t);
            n.chase = Some((mx, mz));
            n.stop = reach * 0.7 + tr;
            n.no_target = 0.0;
            n.pos.yaw = (p.x - mx).atan2(p.z - mz);
            if d <= reach + tr * 0.8 && n.next_attack <= 0.0 && n.info.drawn {
                self.npc_swing(i, now);
            }
            let _ = (id, group);
        }
        for i in done {
            self.npcs.end_hunt(i);
        }
    }

    /// Vendors defend their stall: a mob within reach of the post is fought (and only within `GUARD_LEASH` of it), then the vendor goes back to its post.
    fn vendor_guard(&mut self, dt: f32, now: Instant) {
        let guards: Vec<usize> = self.npcs.npcs.iter().enumerate().filter(|(_, n)| n.role == Role::Vendor && n.lod == Lod::Active && n.down <= 0.0 && n.foe.is_none() && !n.info.weapon.is_empty()).map(|(i, _)| i).collect();
        for i in guards {
            let n = &self.npcs.npcs[i];
            let Some(def) = self.weapons.get(&n.info.weapon).cloned() else { continue };
            let (reach, tr) = (def.reach.unwrap_or(2.1) * WORLD_SCALE, BODY_RADIUS * WORLD_SCALE);
            let post = n.vendor.and_then(|v| self.npcs.economy().vendors.get(v).map(|x| x.post)).unwrap_or((n.pos.x, n.pos.z));
            let p = n.pos;
            let from_post = |m: &crate::mobs::Mob| ((m.pos.x - post.0).powi(2) + (m.pos.z - post.1).powi(2)).sqrt();
            let dist_to = |m: &crate::mobs::Mob| ((m.pos.x - p.x).powi(2) + (m.pos.z - p.z).powi(2)).sqrt();
            let target = self.mobs.candidates(p.x, p.z).map(|k| &self.mobs.mobs[k as usize]).filter(|m| m.alive && dist_to(m) < GUARD_RADIUS && from_post(m) < GUARD_LEASH).min_by(|a, b| dist_to(a).total_cmp(&dist_to(b))).map(|m| m.id);
            let n = &mut self.npcs.npcs[i];
            n.next_attack = (n.next_attack - dt).max(0.0);
            let Some(t) = target else {
                n.target = None;
                n.chase = None;
                continue;
            };
            let m = &self.mobs.mobs[t as usize];
            let (mx, mz, d) = (m.pos.x, m.pos.z, dist_to(m));
            n.target = Some(t);
            n.chase = Some((mx, mz));
            n.stop = reach * 0.7 + tr;
            n.pos.yaw = (p.x - mx).atan2(p.z - mz);
            if d <= reach + tr * 0.8 && n.next_attack <= 0.0 && n.info.drawn {
                self.npc_swing(i, now);
            }
        }
    }

    /// NPC `i` starts a swing: the blow lands at `HIT_AT` of the clip, everybody near sees the animation.
    fn npc_swing(&mut self, i: usize, now: Instant) {
        let n = &mut self.npcs.npcs[i];
        let (id, p, group) = (n.id, n.pos, n.group.clone());
        let dur = self.weapons.clip(swing_clip(&group), 1.0);
        n.next_attack = dur * 0.9 + 0.3;
        self.pending.push(Pending { due: now + Duration::from_secs_f32(dur * HIT_AT), owner: id });
        let msg = serde_json::to_string(&ServerMsg::Swing { id, kind: "melee", clip: swing_clip(&group), group: &group }).unwrap();
        self.send_near(p.x, p.z, msg, None);
    }

    /// How NPCs deal with the players around them. An adventurer's temper (not its race alone) decides: most are friendly, some attack the other side,
    /// a few attack every player. Anybody attacks a player who hurt it lately, and friendly ones greet players of their own side. A fight lasts while the
    /// foe is near, up and the NPC is not badly hurt.
    fn npc_social(&mut self, dt: f32, now: Instant) {
        for n in self.npcs.npcs.iter_mut() {
            n.greet = (n.greet - dt).max(0.0);
            if !n.grudges.is_empty() {
                for g in n.grudges.iter_mut() {
                    g.1 -= dt;
                }
                n.grudges.retain(|g| g.1 > 0.0);
            }
        }
        // players who can be fought: standing in the world, up, not in free-fly
        let people: Vec<(u16, f32, f32, bool, bool)> = self.players.iter().flatten().filter(|p| p.has_pos && p.down_until.is_none() && p.pos.flags & FLAG_FLY == 0).map(|p| (p.info.id, p.pos.x, p.pos.z, chaos_race(&p.info.race), p.pos.flags & FLAG_SNAP != 0)).collect();
        if people.is_empty() {
            for n in self.npcs.npcs.iter_mut() {
                n.foe = None;
                if n.lod == Lod::Active {
                    n.chase = n.chase.filter(|_| matches!(n.goal, Goal::Hunt { fighting: true, .. }));
                }
            }
            return;
        }
        let mut greetings: Vec<(usize, &'static str)> = Vec::new();
        let (tick, pvp) = (self.tick, self.cfg.pvp);
        for (i, n) in self.npcs.npcs.iter_mut().enumerate() {
            if n.lod != Lod::Active || n.down > 0.0 || n.info.weapon.is_empty() {
                continue;
            }
            let chaos = chaos_race(&n.info.race);
            let near = |x: f32, z: f32| ((x - n.pos.x).powi(2) + (z - n.pos.z).powi(2)).sqrt();
            if let Some(pid) = n.foe {
                // keep fighting?
                let foe = people.iter().find(|p| p.0 == pid);
                match foe {
                    Some(&(_, x, z, _, _)) if near(x, z) < 70.0 && n.hp >= n.max_hp * 0.3 => {
                        n.chase = Some((x, z));
                        n.pos.yaw = (n.pos.x - x).atan2(n.pos.z - z);
                    }
                    _ => {
                        n.foe = None;
                        n.chase = None;
                    }
                }
                continue;
            }
            if (tick.wrapping_add(i as u32)) % 8 != 0 {
                continue;
            }
            let busy = matches!(n.goal, Goal::Hunt { fighting: true, .. });
            for &(pid, x, z, pchaos, _) in &people {
                let d = near(x, z);
                let grudge = n.grudges.iter().any(|g| g.0 == pid);
                if pvp && d < NPC_AGGRO && n.hp >= n.max_hp * 0.5 && (grudge || n.hostile_to(pchaos)) {
                    n.foe = Some(pid);
                    n.chase = Some((x, z));
                    break;
                }
                if d < NPC_GREET && n.greet <= 0.0 && !busy && n.temper() != Temper::Outlaw && chaos == pchaos {
                    n.greet = 90.0 + (tick % 60) as f32;
                    greetings.push((i, GREETINGS[(tick as usize / 7 + i) % GREETINGS.len()]));
                    break;
                }
            }
        }
        for (i, text) in greetings {
            let n = &self.npcs.npcs[i];
            let (id, x, z) = (n.id, n.pos.x, n.pos.z);
            let name = n.info.name.clone();
            self.send_near(x, z, serde_json::to_string(&ServerMsg::Chat { id, name: &name, text }).unwrap(), None);
        }
        // the fights: close in on the foe and strike
        let fighting: Vec<usize> = self.npcs.npcs.iter().enumerate().filter(|(_, n)| n.foe.is_some() && n.lod == Lod::Active && n.down <= 0.0).map(|(i, _)| i).collect();
        for i in fighting {
            let n = &mut self.npcs.npcs[i];
            let Some(def) = self.weapons.get(&n.info.weapon).cloned() else { continue };
            let (reach, tr) = (def.reach.unwrap_or(2.1) * WORLD_SCALE, BODY_RADIUS * WORLD_SCALE);
            n.next_attack = (n.next_attack - dt).max(0.0);
            n.stop = reach * 0.7 + tr;
            let Some(c) = n.chase else { continue };
            let d = ((c.0 - n.pos.x).powi(2) + (c.1 - n.pos.z).powi(2)).sqrt();
            if d <= reach + tr * 0.8 && n.next_attack <= 0.0 && n.info.drawn {
                self.npc_swing(i, now);
            }
        }
    }

    /// The blow of `owner`'s swing lands now: the nearest living body inside the weapon's arc takes it.
    fn melee_lands(&mut self, owner: u16) {
        let Some(f) = self.fighter(owner) else { return };
        let group = f.group.clone();
        let Some(def) = self.weapons.get(&f.weapon).cloned() else { return };
        let (reach, arc) = (def.reach.unwrap_or(2.1) * WORLD_SCALE, def.angle.unwrap_or(60.0));
        let (a, yaw, ay) = ((f.pos.x, f.pos.z), f.pos.yaw, f.pos.y);
        let tr = BODY_RADIUS * WORLD_SCALE;
        // an NPC in a fight with a player strikes at nobody else
        let foe = if owner >= NPC_ID_BASE { self.npcs.by_id(owner).and_then(|n| n.foe) } else { None };
        let mut best: Option<(f32, bool, u16)> = None; // (distance, is mob, id)
        for i in self.mobs.candidates(a.0, a.1).filter(|_| foe.is_none()) {
            let m = &self.mobs.mobs[i as usize];
            let t = (m.pos.x, m.pos.z);
            if m.alive && in_arc(a, yaw, t, reach, arc, tr) {
                let d = ((t.0 - a.0).powi(2) + (t.1 - a.1).powi(2)).sqrt();
                if best.map_or(true, |b| d < b.0) {
                    best = Some((d, true, m.id));
                }
            }
        }
        if self.cfg.pvp && owner < NPC_ID_BASE {
            // players can hurt the NPCs they can see
            for n in self.npcs.npcs.iter().filter(|n| n.lod == Lod::Active && n.down <= 0.0) {
                let t = (n.pos.x, n.pos.z);
                if (n.pos.y - ay).abs() < 6.0 && in_arc(a, yaw, t, reach, arc, tr) {
                    let d = ((t.0 - a.0).powi(2) + (t.1 - a.1).powi(2)).sqrt();
                    if best.map_or(true, |b| d < b.0) {
                        best = Some((d, false, n.id));
                    }
                }
            }
        }
        if self.cfg.pvp && (owner < NPC_ID_BASE || foe.is_some()) {
            for q in self.players.iter().flatten().filter(|q| owner < NPC_ID_BASE || Some(q.info.id) == foe) {
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
        let mut dmg = damage(&def, &group, 1.0, (self.tick % 11) as f32 / 10.0);
        if owner >= NPC_ID_BASE {
            dmg = (dmg * self.npcs.damage_factor(owner)).round().max(1.0); // an NPC hits as hard as its weapon skill allows
        }
        let grp = damage_type(&group);
        if is_mob {
            self.hit_mob(owner, vid, dmg, &group);
        } else if vid >= NPC_ID_BASE {
            self.hurt_npc(vid, dmg, (owner, false), &group);
        } else {
            self.hurt_player(vid, dmg, (owner, false), &group, a);
        }
        let _ = grp;
    }

    /// Per-tick combat: mobs think and fight, due blows land, projectiles fly, the fallen get up.
    fn combat(&mut self, dt: f32) {
        let now = Instant::now();
        // players using the free-fly camera are out of the fight (a jump is not: jumping players can still be hit)
        let mut living: Vec<(u16, f32, f32)> = self.players.iter().flatten().filter(|p| p.has_pos && p.down_until.is_none() && p.pos.flags & FLAG_FLY == 0).map(|p| (p.info.id, p.pos.x, p.pos.z)).collect();
        // the NPCs somebody can see are in the fight too: mobs notice them and hit them like players
        living.extend(self.npcs.npcs.iter().filter(|n| n.lod == Lod::Active && n.down <= 0.0).map(|n| (n.id, n.pos.x, n.pos.z)));
        let blows: Vec<MobAttack> = self.mobs.step(dt, &living);
        for b in blows {
            let m = &self.mobs.mobs[b.mob as usize];
            let from = (m.pos.x, m.pos.z);
            if b.player >= NPC_ID_BASE {
                self.hurt_npc(b.player, b.dmg, (b.mob, true), "mob");
            } else {
                self.hurt_player(b.player, b.dmg, (b.mob, true), "mob", from);
            }
        }
        self.npc_fights(dt);
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
            if self.cfg.pvp && s.owner < NPC_ID_BASE {
                for n in self.npcs.npcs.iter().filter(|n| n.lod == Lod::Active && n.down <= 0.0) {
                    if segment_hits(a, b, (n.pos.x, n.pos.z), r, n.pos.y + 1.5) {
                        let d = (n.pos.x - a[0]).powi(2) + (n.pos.z - a[2]).powi(2);
                        if best.map_or(true, |x| d < x.0) {
                            best = Some((d, false, n.id, 0.0));
                        }
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
            } else if vid >= NPC_ID_BASE {
                self.hurt_npc(vid, s.dmg, (s.owner, false), &s.group);
            } else {
                self.hurt_player(vid, s.dmg, (s.owner, false), &s.group, (a[0], a[2]));
            }
            false
        });
        self.shots.append(&mut shots);
        // bodies turn into tombstones, tombstones rot away; loot channels finish
        self.age_corpses(now);
        self.finish_loots(now);
        // get up
        let up: Vec<u16> = self.players.iter().flatten().filter(|p| p.down_until.map_or(false, |t| t <= now)).map(|p| p.info.id).collect();
        for id in up {
            self.respawn(id, true);
        }
    }

    fn chat(&mut self, id: u16, text: &str) {
        let text = clean(text, MAX_CHAT);
        if text.eq_ignore_ascii_case("/logout") {
            self.start_logout(id);
            return;
        }
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
                if p.has_pos {
                    self.gone.insert(id, ([p.pos.x, p.pos.y, p.pos.z, p.pos.yaw], p.down_until.is_some()));
                }
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
        self.age_players(Instant::now());
        self.combat(1.0 / self.cfg.tick_hz as f32);
        let watchers: Vec<(f32, f32)> = self.players.iter().flatten().filter(|p| p.has_pos).map(|p| (p.pos.x, p.pos.z)).collect();
        self.stats.lod = self.npcs.step(1.0 / self.cfg.tick_hz as f32, &watchers, self.cfg.radius);
        // NPCs that just came into view show what they hold; those that got up again are announced to everybody
        let promoted = std::mem::take(&mut self.npcs.promoted);
        for id in promoted {
            let Some(n) = self.npcs.by_id(id) else { continue };
            if !n.info.weapon.is_empty() {
                let (w, drawn, x, z) = (n.info.weapon.clone(), n.info.drawn, n.pos.x, n.pos.z);
                self.send_near(x, z, serde_json::to_string(&ServerMsg::Equip { id, w: &w, drawn }).unwrap(), None);
            }
        }
        let changed: Vec<u16> = std::mem::take(&mut *self.npcs.weapon_changed.borrow_mut());
        for id in changed {
            let Some(n) = self.npcs.by_id(id) else { continue };
            if n.lod == Lod::Active {
                let (w, drawn, x, z) = (n.info.weapon.clone(), n.info.drawn, n.pos.x, n.pos.z);
                self.send_near(x, z, serde_json::to_string(&ServerMsg::Equip { id, w: &w, drawn }).unwrap(), None);
            }
        }
        self.announce_looted();
        let revived = std::mem::take(&mut self.npcs.revived);
        for id in revived {
            let Some(n) = self.npcs.by_id(id) else { continue };
            let (hp, max) = (n.hp, n.max_hp);
            self.broadcast_text(serde_json::to_string(&ServerMsg::Respawn { id, hp, max }).unwrap(), None);
        }
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
        if let Err(e) = store.save(self.npcs.places.len(), self.npcs.vendor_count(), &self.npcs.states()) {
            eprintln!("npc save failed: {e}");
        }
    }

    fn report(&mut self) {
        let s = std::mem::take(&mut self.stats);
        let goals = self.npcs.goal_counts();
        let ((visits, moved), (shelves, carried, cargo)) = self.npcs.trade_stats();
        if visits > 0 || shelves > 0.0 {
            println!("[trade] {visits} visits, {moved:.0} gold changed hands | vendors hold {shelves:.0} gold, travellers carry {carried:.0} gold and {cargo} goods");
        }
        let avg = if s.ticks > 0 { s.tick_us_sum / s.ticks as u128 } else { 0 };
        println!(
            "[stats] {} online | npcs {} active/{} near/{} far, {} rest/{} wander/{} travel/{} to hunt/{} fight/{} explore/{} trade/{} loot | tick avg {} µs max {} µs | out {:.1} KB/s ({:.0} snapshots/s) | dropped {}",
            self.online, s.lod.active, s.lod.near, s.lod.far, goals[0], goals[1], goals[2], goals[3], goals[4], goals[5], goals[6], goals[7], avg, s.tick_us_max, s.bytes_out as f64 / 5.0 / 1024.0, s.snapshots_out as f64 / 5.0, s.dropped
        );
    }
}
