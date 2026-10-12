//! The world task: one task owns every player, so there are no locks. Connection tasks talk to it through `Cmd`s and receive
//! messages through their own bounded channel (a slow client drops position snapshots, and is kicked if it can't keep up with events).
use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;

use crate::proto::*;

#[derive(Clone, Debug)]
pub struct Config {
    /// Players further apart than this (metres, x/z plane) don't receive each other's position updates.
    pub radius: f32,
    pub max_players: usize,
    pub tick_hz: u32,
}

pub struct JoinReq {
    pub name: String,
    pub race: String,
    pub gender: String,
    pub tx: mpsc::Sender<Message>,
    pub reply: oneshot::Sender<Result<u16, &'static str>>,
}

pub enum Cmd {
    Join(JoinReq),
    Pos(u16, PosUpdate),
    Chat(u16, String),
    Leave(u16),
}

struct Player {
    info: PlayerInfo,
    pos: PosUpdate,
    has_pos: bool,
    tx: mpsc::Sender<Message>,
    last_chat: Instant,
    last_sent: usize,
    dead: bool,
}

#[derive(Default)]
struct Stats {
    ticks: u32,
    tick_us_sum: u128,
    tick_us_max: u128,
    bytes_out: u64,
    snapshots_out: u64,
    dropped: u64,
}

struct World {
    cfg: Config,
    players: Vec<Option<Player>>,
    online: usize,
    cursor: usize,
    tick: u32,
    stats: Stats,
}

pub async fn run(cfg: Config, mut rx: mpsc::Receiver<Cmd>) {
    let mut world = World { players: (0..cfg.max_players).map(|_| None).collect(), cfg: cfg.clone(), online: 0, cursor: 0, tick: 0, stats: Stats::default() };
    let mut ticker = interval(Duration::from_secs_f64(1.0 / cfg.tick_hz as f64));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut report = interval(Duration::from_secs(5));
    report.tick().await;
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd { Some(c) => world.handle(c), None => break },
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
}

impl World {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Join(req) => self.join(req),
            Cmd::Pos(id, p) => {
                if let Some(Some(pl)) = self.players.get_mut(id as usize) {
                    pl.pos = p;
                    pl.has_pos = true;
                }
            }
            Cmd::Chat(id, text) => self.chat(id, &text),
            Cmd::Leave(id) => self.remove(id),
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
        let info = PlayerInfo { id: id as u16, name, race: req.race, gender: req.gender };
        self.players[id] = Some(Player { info, pos: PosUpdate::default(), has_pos: false, tx: req.tx, last_chat: Instant::now() - Duration::from_secs(10), last_sent: 0, dead: false });
        self.online += 1;
        let _ = req.reply.send(Ok(id as u16));

        let me = self.players[id].as_ref().unwrap();
        let all: Vec<&PlayerInfo> = self.players.iter().flatten().map(|p| &p.info).collect();
        let welcome = serde_json::to_string(&ServerMsg::Welcome { id: id as u16, radius: self.cfg.radius, tick_hz: self.cfg.tick_hz, players: all }).unwrap();
        if me.tx.try_send(Message::Text(welcome)).is_err() {
            self.players[id].as_mut().unwrap().dead = true;
            return;
        }
        let joined = ServerMsg::Joined { player: &me.info };
        let text = serde_json::to_string(&joined).unwrap();
        self.broadcast_text(text, Some(id as u16));
        println!("+ {} ({}_{}) id {} — {} online", self.players[id].as_ref().unwrap().info.name, self.players[id].as_ref().unwrap().info.race, self.players[id].as_ref().unwrap().info.gender, id, self.online);
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
        let cell = self.cfg.radius.max(1.0);
        let r2 = self.cfg.radius * self.cfg.radius;
        let ents: Vec<(u16, PosUpdate)> = self.players.iter().flatten().filter(|p| p.has_pos).map(|p| (p.info.id, p.pos)).collect();
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
            if count == 0 && p.last_sent == 0 {
                continue; // nobody around now or last time: nothing to say
            }
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
        self.reap();
    }

    fn report(&mut self) {
        let s = std::mem::take(&mut self.stats);
        let avg = if s.ticks > 0 { s.tick_us_sum / s.ticks as u128 } else { 0 };
        println!(
            "[stats] {} online | tick avg {} µs max {} µs | out {:.1} KB/s ({:.0} snapshots/s) | dropped {}",
            self.online, avg, s.tick_us_max, s.bytes_out as f64 / 5.0 / 1024.0, s.snapshots_out as f64 / 5.0, s.dropped
        );
    }
}
