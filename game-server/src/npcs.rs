//! Server-owned living NPCs. They are labeled (`PlayerInfo::npc`) avatars that travel between the world's towns, rest there and wander about.
//!
//! The cost of 500+ of them is kept flat by simulation level of detail (LOD), decided from the distance to the nearest player:
//! * `Active` (within the interest radius): stepped every tick and sent in the player snapshots,
//! * `Near` (within `NEAR_RADIUS`): stepped every `NEAR_EVERY` seconds,
//! * `Far`: stepped every `FAR_EVERY` seconds.
//!
//! A step moves the NPC along its current goal by the accumulated time, so a far NPC covers (and may finish) whole legs in one go, and an NPC that
//! gets close is caught up to "now" first, so it never appears to jump. Travel is a hop along a graph that joins every place to its nearest neighbours
//! (a stand-in for a road network). Positions use the wire convention (viewer three.js space: game z negated); height is left to the clients (`FLAG_SNAP`).
use std::collections::HashMap;

use serde::Deserialize;

use crate::proto::{PlayerInfo, PosUpdate, FLAG_NPC, FLAG_SNAP, GENDERS, RACES};

/// NPC ids live above every player slot (`world::MAX_PLAYERS`).
pub const NPC_ID_BASE: u16 = 0xC000;
pub const MAX_NPCS: usize = 0x4000;
/// Seconds between steps of an NPC that is within NEAR_RADIUS / further away.
pub const NEAR_EVERY: f32 = 0.5;
pub const FAR_EVERY: f32 = 5.0;
pub const NEAR_RADIUS: f32 = 3000.0;
const WALK_SPEED: f32 = 1.4;
const TRAVEL_SPEED: f32 = 4.2;
/// A place's graph neighbours (nearest places), and the longest hop a traveller will take.
const LINKS: usize = 5;
const MAX_HOP: f32 = 6000.0;
const GEAR: [&str; 8] = ["", "Cloth", "Leather", "Chain", "Plate", "Banded", "Bone", "Dragon"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lod {
    Far,
    Near,
    Active,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    /// Sit in town for this many more seconds.
    Rest { left: f32 },
    /// Potter about within `WANDER_R` of the place `at` for this many more seconds, towards `to` (wire x/z) when there is one.
    Wander { left: f32, to: Option<(f32, f32)> },
    /// Walk to place `dest`.
    Travel { dest: usize },
}

const WANDER_R: f32 = 45.0;

#[derive(Clone, Debug)]
pub struct Place {
    pub name: String,
    /// Wire coordinates (z negated).
    pub x: f32,
    pub z: f32,
    /// A settlement NPCs live in (as opposed to a dungeon / bank they only visit).
    pub town: bool,
}

#[derive(Clone, Debug)]
pub struct Npc {
    pub id: u16,
    pub info: PlayerInfo,
    pub pos: PosUpdate,
    pub lod: Lod,
    pub level: u16,
    /// Place the NPC lives in / last arrived at.
    pub home: usize,
    pub at: usize,
    pub goal: Goal,
    /// 0..1, grows while travelling, shrinks while resting.
    pub fatigue: f32,
    /// Seconds since the last step.
    acc: f32,
    rng: u32,
}

/// The persisted part of an NPC (see `store.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct NpcState {
    pub idx: u32,
    pub name: String,
    pub race: String,
    pub gender: String,
    pub gear: String,
    pub level: u16,
    pub home: usize,
    pub at: usize,
    /// 0 rest, 1 wander, 2 travel; `goal_a` is the time left, `goal_dest` the place travelled to (-1 otherwise).
    pub goal_kind: u8,
    pub goal_a: f32,
    pub goal_dest: i64,
    pub fatigue: f32,
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub rng: u32,
}

#[derive(Deserialize)]
struct PoiFile {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    x: f32,
    z: f32,
}

#[derive(Deserialize)]
struct BindFile {
    name: String,
    race: String,
    x: f32,
    z: f32,
}

#[derive(Default)]
pub struct Npcs {
    pub npcs: Vec<Npc>,
    pub places: Vec<Place>,
    links: Vec<Vec<usize>>,
    /// Race that rules the capital nearest to each place.
    race_of: Vec<usize>,
    tick: u32,
}

/// What the LOD pass did in one tick (for the stats line).
#[derive(Default, Clone, Copy, Debug)]
pub struct LodCounts {
    pub active: u32,
    pub near: u32,
    pub far: u32,
}

fn xorshift(s: &mut u32) -> u32 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    x
}

fn frac(s: &mut u32) -> f32 {
    (xorshift(s) >> 8) as f32 / (1u32 << 24) as f32
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

const SYL_A: [&str; 16] = ["Ka", "Ur", "Bel", "Tho", "Mar", "Sil", "Dra", "Vor", "Ela", "Gor", "Nes", "Ith", "Rha", "Zul", "Oru", "Fen"];
const SYL_B: [&str; 12] = ["dan", "mir", "gar", "wen", "thas", "rik", "lor", "nus", "bek", "dra", "val", "sen"];

pub fn make_name(rng: &mut u32) -> String {
    format!("{}{}", SYL_A[xorshift(rng) as usize % SYL_A.len()], SYL_B[xorshift(rng) as usize % SYL_B.len()])
}

fn race_code(name: &str) -> Option<usize> {
    let code = match name {
        "Human" => "hum",
        "Alfar" => "alf",
        "Dwarf" => "dwf",
        "Ork" => "ork",
        "Mirdain" => "mir",
        "Mahirim" => "mhm",
        _ => return None,
    };
    RACES.iter().position(|r| *r == code)
}

impl Npcs {
    pub fn empty() -> Self {
        Npcs::default()
    }

    /// Build the place graph from `pois.json` (towns, dungeons) and `spawns.json` (the racial capitals), then create `count` NPCs.
    /// `seed` makes the population reproducible (a saved world overrides it with the stored state, see `restore`).
    pub fn load(pois: &str, bindstones: &str, count: usize, seed: u32) -> Result<Self, String> {
        let pois: Vec<PoiFile> = serde_json::from_str(&std::fs::read_to_string(pois).map_err(|e| format!("{pois}: {e}"))?).map_err(|e| e.to_string())?;
        let binds: Vec<BindFile> = serde_json::from_str(&std::fs::read_to_string(bindstones).map_err(|e| format!("{bindstones}: {e}"))?).map_err(|e| e.to_string())?;
        let mut places: Vec<Place> = binds.iter().map(|b| Place { name: b.name.clone(), x: b.x, z: -b.z, town: true }).collect();
        for p in &pois {
            let town = matches!(p.kind.as_str(), "Village" | "Player Hamlet" | "Player City" | "NPC City" | "Chaos City" | "Fortress");
            if town || p.kind == "Dungeon" {
                places.push(Place { name: p.name.clone(), x: p.x, z: -p.z, town });
            }
        }
        let capitals: Vec<(usize, (f32, f32))> = binds.iter().filter_map(|b| race_code(&b.race).map(|r| (r, (b.x, -b.z)))).collect();
        Ok(Self::build(places, &capitals, count, seed))
    }

    /// Same as `load` from places already in memory (tests): `capitals` are (race index, wire x/z).
    pub fn build(places: Vec<Place>, capitals: &[(usize, (f32, f32))], count: usize, seed: u32) -> Self {
        let links: Vec<Vec<usize>> = (0..places.len())
            .map(|i| {
                let mut near: Vec<(f32, usize)> = (0..places.len()).filter(|&j| j != i).map(|j| (dist((places[i].x, places[i].z), (places[j].x, places[j].z)), j)).filter(|d| d.0 <= MAX_HOP).collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                near.into_iter().take(LINKS).map(|d| d.1).collect()
            })
            .collect();
        let race_of: Vec<usize> = places.iter().map(|p| capitals.iter().min_by(|a, b| dist((p.x, p.z), a.1).total_cmp(&dist((p.x, p.z), b.1))).map_or(0, |c| c.0)).collect();
        let towns: Vec<usize> = (0..places.len()).filter(|&i| places[i].town).collect();
        let mut me = Npcs { npcs: Vec::new(), places, links, race_of, tick: 0 };
        if towns.is_empty() {
            return me;
        }
        let mut rng = seed | 1;
        for i in 0..count.min(MAX_NPCS) {
            let home = towns[xorshift(&mut rng) as usize % towns.len()];
            let (race, gender) = (me.race_of[home], xorshift(&mut rng) as usize % 2);
            let name = make_name(&mut rng);
            let gear = GEAR[xorshift(&mut rng) as usize % GEAR.len()].to_string();
            let p = &me.places[home];
            let (x, z) = (p.x + (frac(&mut rng) - 0.5) * 2.0 * WANDER_R, p.z + (frac(&mut rng) - 0.5) * 2.0 * WANDER_R);
            let mut npc = Npc {
                id: NPC_ID_BASE + i as u16,
                info: PlayerInfo { id: NPC_ID_BASE + i as u16, name, race: RACES[race].to_string(), gender: GENDERS[gender].to_string(), gear, weapon: String::new(), drawn: false, npc: true },
                pos: PosUpdate { x, y: 0.0, z, yaw: frac(&mut rng) * 6.28, speed: 0, flags: FLAG_SNAP | FLAG_NPC },
                lod: Lod::Far,
                level: 1 + (xorshift(&mut rng) % 20) as u16,
                home,
                at: home,
                goal: Goal::Rest { left: frac(&mut rng) * 30.0 },
                fatigue: frac(&mut rng) * 0.5,
                acc: frac(&mut rng) * FAR_EVERY,
                rng: xorshift(&mut rng) | 1,
            };
            me.think(&mut npc);
            me.npcs.push(npc);
        }
        me
    }

    pub fn by_id(&self, id: u16) -> Option<&Npc> {
        id.checked_sub(NPC_ID_BASE).and_then(|i| self.npcs.get(i as usize))
    }

    /// Pick the next goal of `n` from its needs (utility scores; all stubs for now): tired -> rest; otherwise mostly travel to a neighbouring
    /// place, sometimes potter around where it is.
    fn think(&self, n: &mut Npc) {
        let links = &self.links[n.at];
        let rest = n.fatigue * 1.4;
        let wander = 0.25 + frac(&mut n.rng) * 0.2;
        let travel = if links.is_empty() { 0.0 } else { 0.3 + frac(&mut n.rng) * 0.6 };
        let in_town = self.places[n.at].town;
        n.goal = if rest > wander.max(travel) && in_town {
            Goal::Rest { left: 15.0 + n.fatigue * 90.0 * (0.5 + frac(&mut n.rng)) }
        } else if travel >= wander {
            // a dungeon is a place to look at, not to live in: towns are the more likely stop
            let towns: Vec<usize> = links.iter().copied().filter(|&p| self.places[p].town).collect();
            let pool = if !towns.is_empty() && frac(&mut n.rng) < 0.75 { &towns } else { links };
            Goal::Travel { dest: pool[xorshift(&mut n.rng) as usize % pool.len()] }
        } else {
            Goal::Wander { left: 10.0 + frac(&mut n.rng) * 30.0, to: None }
        };
    }

    /// Move `n` along its goal by `dt` seconds (any amount: legs finish and new goals start inside the loop).
    fn advance(&self, n: &mut Npc, mut dt: f32) {
        let mut guard = 0;
        while dt > 1e-4 && guard < 64 {
            guard += 1;
            match n.goal {
                Goal::Rest { left } => {
                    let used = left.min(dt);
                    n.fatigue = (n.fatigue - used / 120.0).max(0.0);
                    n.pos.speed = 0;
                    dt -= used;
                    if used >= left {
                        self.think(n);
                    } else {
                        n.goal = Goal::Rest { left: left - used };
                    }
                }
                Goal::Wander { left, to } => {
                    let p = &self.places[n.at];
                    let target = to.unwrap_or_else(|| {
                        let a = frac(&mut n.rng) * 6.283;
                        let r = frac(&mut n.rng) * WANDER_R;
                        (p.x + a.cos() * r, p.z + a.sin() * r)
                    });
                    let d = dist((n.pos.x, n.pos.z), target);
                    let can = dt.min(left);
                    let arrived = d <= WALK_SPEED * can;
                    let used = if arrived { d / WALK_SPEED } else { can };
                    if arrived {
                        n.pos.x = target.0;
                        n.pos.z = target.1;
                        n.pos.speed = 0;
                    } else {
                        n.pos.yaw = (target.0 - n.pos.x).atan2(target.1 - n.pos.z);
                        n.pos.x += (target.0 - n.pos.x) / d * WALK_SPEED * used;
                        n.pos.z += (target.1 - n.pos.z) / d * WALK_SPEED * used;
                        n.pos.speed = (WALK_SPEED * 10.0) as u8;
                    }
                    dt -= used;
                    if left - used <= 1e-3 {
                        n.pos.speed = 0;
                        self.think(n);
                    } else {
                        n.goal = Goal::Wander { left: left - used, to: if arrived { None } else { Some(target) } };
                    }
                }
                Goal::Travel { dest } => {
                    let p = &self.places[dest];
                    let target = (p.x, p.z);
                    let d = dist((n.pos.x, n.pos.z), target);
                    let reach = TRAVEL_SPEED * dt;
                    if d <= reach {
                        n.fatigue = (n.fatigue + d / 9000.0).min(1.0);
                        dt -= d / TRAVEL_SPEED;
                        n.pos.x = target.0;
                        n.pos.z = target.1;
                        n.pos.speed = 0;
                        n.at = dest;
                        self.think(n);
                    } else {
                        n.pos.yaw = (target.0 - n.pos.x).atan2(target.1 - n.pos.z);
                        n.pos.x += (target.0 - n.pos.x) / d * reach;
                        n.pos.z += (target.1 - n.pos.z) / d * reach;
                        n.pos.speed = (TRAVEL_SPEED * 10.0) as u8;
                        n.fatigue = (n.fatigue + reach / 9000.0).min(1.0);
                        dt = 0.0;
                    }
                }
            }
        }
    }

    /// One server tick: decide every NPC's LOD from the players' (wire x/z) positions and step those that are due.
    /// `radius` is the players' interest radius (Active). Returns how many NPCs are at each level.
    pub fn step(&mut self, dt: f32, players: &[(f32, f32)], radius: f32) -> LodCounts {
        self.tick = self.tick.wrapping_add(1);
        let cell = NEAR_RADIUS;
        let mut grid: HashMap<(i32, i32), Vec<(f32, f32)>> = HashMap::new();
        for &(x, z) in players {
            grid.entry(((x / cell).floor() as i32, (z / cell).floor() as i32)).or_default().push((x, z));
        }
        let mut counts = LodCounts::default();
        let mut npcs = std::mem::take(&mut self.npcs);
        for (i, n) in npcs.iter_mut().enumerate() {
            // a far NPC re-checks its distance only now and then; one that is already close checks every tick
            if n.lod == Lod::Far && (i as u32 + self.tick) % 15 != 0 {
                n.acc += dt;
                if n.acc >= FAR_EVERY {
                    let a = std::mem::take(&mut n.acc);
                    self.advance(n, a);
                }
                counts.far += 1;
                continue;
            }
            let (cx, cz) = ((n.pos.x / cell).floor() as i32, (n.pos.z / cell).floor() as i32);
            let mut best = f32::MAX;
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&(cx + dx, cz + dz)) {
                        for &q in list {
                            best = best.min(dist((n.pos.x, n.pos.z), q));
                        }
                    }
                }
            }
            let lod = if best <= radius { Lod::Active } else if best <= NEAR_RADIUS { Lod::Near } else { Lod::Far };
            n.acc += dt;
            if lod == Lod::Active && n.lod != Lod::Active {
                // catch up to "now" before anybody sees it
                let a = std::mem::take(&mut n.acc);
                self.advance(n, a);
            }
            n.lod = lod;
            let every = match lod {
                Lod::Active => 0.0,
                Lod::Near => NEAR_EVERY,
                Lod::Far => FAR_EVERY,
            };
            if n.acc >= every {
                let a = std::mem::take(&mut n.acc);
                self.advance(n, a);
            }
            match lod {
                Lod::Active => counts.active += 1,
                Lod::Near => counts.near += 1,
                Lod::Far => counts.far += 1,
            }
        }
        self.npcs = npcs;
        counts
    }

    /// NPCs the players can currently see (sent in the player snapshots).
    pub fn active(&self) -> impl Iterator<Item = &Npc> {
        self.npcs.iter().filter(|n| n.lod == Lod::Active)
    }

    /// Everything about every NPC that has to survive a restart.
    pub fn states(&self) -> Vec<NpcState> {
        self.npcs
            .iter()
            .map(|n| {
                let (kind, a, dest) = match n.goal {
                    Goal::Rest { left } => (0, left, -1),
                    Goal::Wander { left, .. } => (1, left, -1),
                    Goal::Travel { dest } => (2, 0.0, dest as i64),
                };
                NpcState { idx: (n.id - NPC_ID_BASE) as u32, name: n.info.name.clone(), race: n.info.race.clone(), gender: n.info.gender.clone(), gear: n.info.gear.clone(), level: n.level, home: n.home, at: n.at, goal_kind: kind, goal_a: a, goal_dest: dest, fatigue: n.fatigue, x: n.pos.x, z: n.pos.z, yaw: n.pos.yaw, rng: n.rng }
            })
            .collect()
    }

    /// Put saved NPCs back (rows that don't fit the current population or place list are skipped). Returns how many were restored.
    pub fn restore(&mut self, states: &[NpcState]) -> usize {
        let places = self.places.len();
        let mut restored = 0;
        for st in states {
            let Some(n) = self.npcs.get_mut(st.idx as usize) else { continue };
            let goal = match st.goal_kind {
                0 => Goal::Rest { left: st.goal_a },
                1 => Goal::Wander { left: st.goal_a, to: None },
                2 if st.goal_dest >= 0 && (st.goal_dest as usize) < places => Goal::Travel { dest: st.goal_dest as usize },
                _ => continue,
            };
            if st.home >= places || st.at >= places || !st.x.is_finite() || !st.z.is_finite() {
                continue;
            }
            n.info.name = st.name.clone();
            n.info.race = st.race.clone();
            n.info.gender = st.gender.clone();
            n.info.gear = st.gear.clone();
            n.level = st.level;
            n.home = st.home;
            n.at = st.at;
            n.goal = goal;
            n.fatigue = st.fatigue.clamp(0.0, 1.0);
            n.pos.x = st.x;
            n.pos.z = st.z;
            n.pos.yaw = st.yaw;
            n.pos.speed = 0;
            n.rng = st.rng | 1;
            n.acc = 0.0;
            restored += 1;
        }
        restored
    }

    pub fn infos(&self) -> impl Iterator<Item = &PlayerInfo> {
        self.npcs.iter().map(|n| &n.info)
    }
}
