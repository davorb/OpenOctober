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
use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::combat::{is_melee, mob_skill, npc_hp, skill_damage, weapon_skill, Weapons};
use crate::nav::Nav;
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
    /// Hunt mobs at the spawn point `ground` for this many more seconds: first walk there, then fight (`fighting`). Far from any player the fight is
    /// only time and experience; an active NPC really fights the mobs (the world drives that, see `world.rs`).
    Hunt { ground: usize, left: f32, fighting: bool },
}

const WANDER_R: f32 = 45.0;
/// An NPC hunts at spawn points this close to the place it is at, and no tougher than `hunt_cap(level)`.
const HUNT_RADIUS: f32 = 2500.0;
const GROUND_CELL: f32 = 2000.0;
/// Seconds a dead NPC lies there before it is back at its home place.
pub const DOWN_SECS: f32 = 6.0;

/// The toughest mob rank (see `combat::difficulty_rank`) an NPC with this weapon skill dares to hunt.
pub fn hunt_cap(skill: f32) -> u8 {
    (skill / 20.0).floor().clamp(0.0, 4.0) as u8
}

/// Skill gained from one use (a blow landed, or one taken: `per` is the gain at skill 0) against a mob of `rank`: the more the skill is past
/// what the mob is a fair fight for, the less it teaches, and training slows towards 100.
pub fn skill_gain(skill: f32, rank: u8, per: f32) -> f32 {
    per * (1.0 - (skill - mob_skill(rank)).max(0.0) / 30.0).max(0.0) * (1.0 - skill / 130.0).max(0.0)
}

/// An active NPC plans its route this far ahead at a time, searching at most this many cells.
const ROUTE_LEG: f32 = 250.0;
const ROUTE_EXPAND: usize = 60000;

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
    /// Skills (0..100) by their name in the game's skill list ('Swords', 'Health', ...). Skills are raised by use, there are no levels.
    pub skills: BTreeMap<String, f32>,
    pub hp: f32,
    pub max_hp: f32,
    /// Weapon class of `info.weapon` ("" = unarmed: such an NPC does not hunt).
    pub group: String,
    /// Seconds left lying dead (0 = alive), until the next blow may be struck.
    pub down: f32,
    pub next_attack: f32,
    /// Mob being fought, the point to walk to for it, how close to get, and seconds without a target (the world sets these each tick).
    pub target: Option<u16>,
    pub chase: Option<(f32, f32)>,
    pub stop: f32,
    pub no_target: f32,
    /// The weapon skill the NPC starts with in whatever class it is armed with (`equip_all`).
    start_melee: f32,
    /// Place the NPC lives in / last arrived at.
    pub home: usize,
    pub at: usize,
    pub goal: Goal,
    /// 0..1, grows while travelling, shrinks while resting.
    pub fatigue: f32,
    /// Seconds since the last step.
    acc: f32,
    rng: u32,
    /// Waypoints still to walk (active NPCs only) and the goal point they lead to.
    path: Vec<(f32, f32)>,
    path_goal: Option<(f32, f32)>,
    /// Last position (active NPCs) known to be walkable.
    good: (f32, f32),
}

/// The persisted part of an NPC (see `store.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct NpcState {
    pub idx: u32,
    pub name: String,
    pub race: String,
    pub gender: String,
    pub gear: String,
    /// The skills as JSON ({"Swords": 12.3, ...}).
    pub skills: String,
    pub weapon: String,
    pub home: usize,
    pub at: usize,
    /// 0 rest, 1 wander, 2 travel, 3 hunt; `goal_a` is the time left, `goal_dest` the place travelled to / the hunting ground (-1 otherwise).
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
    /// Hunting grounds (mob spawn points: x, z, rank of the toughest mob) and a coarse index of them.
    pub grounds: Vec<(f32, f32, u8)>,
    ground_grid: HashMap<(i32, i32), Vec<usize>>,
    /// Ids that became visible / came back to life in the last `step` (the world announces them).
    pub promoted: Vec<u16>,
    pub revived: Vec<u16>,
    /// Walkability of the world: active NPCs path-find on it and stand at its terrain height.
    pub nav: Option<Nav>,
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

impl Npc {
    pub fn skill(&self, name: &str) -> f32 {
        self.skills.get(name).copied().unwrap_or(0.0)
    }

    /// The skill of the weapon class it is armed with.
    pub fn melee_skill(&self) -> f32 {
        self.skill(weapon_skill(&self.group))
    }

    /// The toughest mob rank it dares to hunt.
    pub fn hunt_cap(&self) -> u8 {
        hunt_cap(self.melee_skill())
    }
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
        let mut me = Npcs { npcs: Vec::new(), places, links, race_of, tick: 0, grounds: Vec::new(), ground_grid: HashMap::new(), promoted: Vec::new(), revived: Vec::new(), nav: None };
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
                skills: BTreeMap::from([("Health".to_string(), 10.0 + frac(&mut rng) * 50.0)]),
                hp: 0.0,
                max_hp: 0.0,
                group: String::new(),
                down: 0.0,
                next_attack: 0.0,
                target: None,
                chase: None,
                stop: 2.0,
                no_target: 0.0,
                start_melee: 5.0 + frac(&mut rng) * frac(&mut rng) * 65.0,
                home,
                at: home,
                goal: Goal::Rest { left: frac(&mut rng) * 30.0 },
                fatigue: frac(&mut rng) * 0.5,
                acc: frac(&mut rng) * FAR_EVERY,
                rng: xorshift(&mut rng) | 1,
                path: Vec::new(),
                path_goal: None,
                good: (x, z),
            };
            npc.max_hp = npc_hp(npc.skill("Health"));
            npc.hp = npc.max_hp;
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
        let hunt = if n.group.is_empty() || n.fatigue > 0.6 || n.hp < n.max_hp * 0.5 { None } else { self.hunting_ground(n).map(|g| (g, 0.35 + frac(&mut n.rng) * 0.55)) };
        n.goal = if let Some((ground, _)) = hunt.filter(|h| h.1 >= wander.max(travel) && h.1 > rest) {
            Goal::Hunt { ground, left: 60.0 + frac(&mut n.rng) * 120.0, fighting: false }
        } else if rest > wander.max(travel) && in_town {
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

    /// A random hunting ground near the place `n` is at that is not too tough for it.
    fn hunting_ground(&self, n: &mut Npc) -> Option<usize> {
        let p = &self.places[n.at];
        let (cx, cz) = ((p.x / GROUND_CELL).floor() as i32, (p.z / GROUND_CELL).floor() as i32);
        let cap = n.hunt_cap();
        let mut pool: Vec<usize> = Vec::new();
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &g in self.ground_grid.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    let gr = self.grounds[g];
                    if gr.2 <= cap && dist((p.x, p.z), (gr.0, gr.1)) <= HUNT_RADIUS {
                        pool.push(g);
                    }
                }
            }
        }
        (!pool.is_empty()).then(|| pool[xorshift(&mut n.rng) as usize % pool.len()])
    }

    /// Give the NPCs somewhere to hunt (`mobs.grounds()`), and think again: a fresh population picks its first goals knowing about them.
    pub fn set_grounds(&mut self, grounds: Vec<(f32, f32, u8)>) {
        self.ground_grid.clear();
        for (i, g) in grounds.iter().enumerate() {
            self.ground_grid.entry(((g.0 / GROUND_CELL).floor() as i32, (g.1 / GROUND_CELL).floor() as i32)).or_default().push(i);
        }
        self.grounds = grounds;
        let mut npcs = std::mem::take(&mut self.npcs);
        for n in npcs.iter_mut() {
            self.think(n);
        }
        self.npcs = npcs;
    }

    /// Arm every NPC with a melee weapon from the table that suits its level (a rank-10 weapon at level 1, a rank-80 one at level 20).
    pub fn equip_all(&mut self, weapons: &Weapons) {
        let mut ids: Vec<(&String, &crate::combat::WeaponDef)> = weapons.defs.iter().filter(|(_, d)| d.rank.is_some() && d.group.as_deref().map_or(false, |g| is_melee(g) && g != "shield")).collect();
        ids.sort_by(|a, b| a.0.cmp(b.0));
        if ids.is_empty() {
            return;
        }
        for n in self.npcs.iter_mut() {
            let max_rank = 10.0 + n.start_melee * 0.7;
            let fit: Vec<_> = ids.iter().filter(|(_, d)| d.rank.unwrap_or(99.0) <= max_rank).collect();
            let pool: Vec<_> = if fit.is_empty() { ids.iter().collect() } else { fit };
            let (id, def) = pool[xorshift(&mut n.rng) as usize % pool.len()];
            n.info.weapon = (*id).clone();
            n.group = def.group.clone().unwrap_or_default();
            let start = n.start_melee;
            n.skills.entry(weapon_skill(&n.group).to_string()).or_insert(start);
        }
    }

    /// Raise a skill of `n` by `amount` (capped at 100); Health also raises the hit points.
    fn train(&self, n: &mut Npc, skill: &str, amount: f32) {
        let v = n.skills.entry(skill.to_string()).or_insert(0.0);
        *v = (*v + amount).min(100.0);
        if skill == "Health" {
            let max = npc_hp(*v);
            n.hp += max - n.max_hp;
            n.max_hp = max;
        }
    }

    /// An NPC landed a blow on a mob of this rank: its weapon skill trains.
    pub fn train_hit(&mut self, id: u16, rank: u8) {
        let Some(i) = id.checked_sub(NPC_ID_BASE) else { return };
        let mut npcs = std::mem::take(&mut self.npcs);
        if let Some(n) = npcs.get_mut(i as usize) {
            let skill = weapon_skill(&n.group);
            let gain = skill_gain(n.skill(skill), rank, 0.12);
            self.train(n, skill, gain);
        }
        self.npcs = npcs;
    }

    /// An NPC took a blow: its Health trains (a little less the tougher it already is).
    pub fn train_hurt(&mut self, id: u16) {
        let Some(i) = id.checked_sub(NPC_ID_BASE) else { return };
        let mut npcs = std::mem::take(&mut self.npcs);
        if let Some(n) = npcs.get_mut(i as usize) {
            let gain = skill_gain(n.skill("Health"), 2, 0.06);
            self.train(n, "Health", gain);
        }
        self.npcs = npcs;
    }

    /// Multiplier on the damage of an NPC's blows from its weapon skill.
    pub fn damage_factor(&self, id: u16) -> f32 {
        self.by_id(id).map_or(1.0, |n| skill_damage(n.melee_skill()))
    }

    /// Stop hunting (hurt, or nothing left to fight): choose something else to do.
    pub fn end_hunt(&mut self, idx: usize) {
        let mut npcs = std::mem::take(&mut self.npcs);
        if let Some(n) = npcs.get_mut(idx) {
            self.finish_hunt(n);
        }
        self.npcs = npcs;
    }

    fn finish_hunt(&self, n: &mut Npc) {
        n.chase = None;
        n.target = None;
        n.no_target = 0.0;
        n.pos.speed = 0;
        n.fatigue = (n.fatigue + 0.15).min(1.0);
        self.think(n);
    }

    /// Back to life at the home place.
    fn revive(&self, n: &mut Npc) {
        n.down = 0.0;
        n.hp = n.max_hp;
        n.target = None;
        n.chase = None;
        n.path.clear();
        n.path_goal = None;
        n.at = n.home;
        let p = &self.places[n.home];
        n.pos.x = p.x;
        n.pos.z = p.z;
        n.pos.speed = 0;
        n.good = (p.x, p.z);
        n.goal = Goal::Rest { left: 5.0 };
        n.lod = Lod::Far;
    }

    /// Walk `n` towards `target` for up to `dt` seconds at `speed`. An active NPC follows a route over the nav grid (planned a leg of `ROUTE_LEG`
    /// metres at a time, re-planned when it runs out); anyone else just walks the straight line. Returns (seconds used, arrived at `target`).
    /// `None` = no route: the caller gives up on the goal.
    fn walk(&self, n: &mut Npc, target: (f32, f32), speed: f32, dt: f32, active: bool) -> Option<(f32, bool)> {
        let mut point = target;
        if let (true, Some(nav)) = (active, &self.nav) {
            let here = (n.pos.x, n.pos.z);
            if n.path_goal != Some(target) || n.path.is_empty() {
                let d = dist(here, target);
                if d < 1.0 {
                    n.path.clear();
                } else {
                    let leg = if d > ROUTE_LEG { (here.0 + (target.0 - here.0) / d * ROUTE_LEG, here.1 + (target.1 - here.1) / d * ROUTE_LEG) } else { target };
                    n.path = nav.find_path(here, leg, ROUTE_EXPAND)?;
                    n.path_goal = Some(target);
                }
            }
            while n.path.first().map_or(false, |&w| dist(here, w) < 0.5) {
                n.path.remove(0);
            }
            point = n.path.first().copied().unwrap_or(target);
        }
        let d = dist((n.pos.x, n.pos.z), point);
        let can = speed * dt;
        if d > 1e-4 {
            n.pos.yaw = (n.pos.x - point.0).atan2(n.pos.z - point.1); // the players' convention: forward = (-sin yaw, -cos yaw)
        }
        if d <= can {
            n.pos.x = point.0;
            n.pos.z = point.1;
            if !n.path.is_empty() {
                n.path.remove(0);
            }
            let arrived = point == target && n.path.is_empty();
            n.pos.speed = if arrived { 0 } else { (speed * 10.0) as u8 };
            if arrived {
                n.path_goal = None;
            }
            Some((d / speed, arrived))
        } else {
            n.pos.x += (point.0 - n.pos.x) / d * can;
            n.pos.z += (point.1 - n.pos.z) / d * can;
            n.pos.speed = (speed * 10.0) as u8;
            Some((dt, false))
        }
    }

    /// Where an active NPC actually goes to for `target`: the nearest free cell (a town's centre may be inside a building).
    fn standable(&self, target: (f32, f32), active: bool) -> (f32, f32) {
        match (&self.nav, active) {
            (Some(nav), true) => nav.snap(target, 12).unwrap_or(target),
            _ => target,
        }
    }

    /// The goal can't be reached (no route): wait a moment, then think again.
    fn give_up(&self, n: &mut Npc) {
        n.path.clear();
        n.path_goal = None;
        n.pos.speed = 0;
        n.goal = Goal::Rest { left: 3.0 + frac(&mut n.rng) * 4.0 };
    }

    /// Move `n` along its goal by `dt` seconds (any amount: legs finish and new goals start inside the loop).
    fn advance(&self, n: &mut Npc, mut dt: f32, active: bool) {
        if n.hp < n.max_hp && !matches!(n.goal, Goal::Hunt { fighting: true, .. }) {
            n.hp = (n.hp + n.max_hp * 0.02 * dt).min(n.max_hp);
        }
        let mut guard = 0;
        while dt > 1e-4 && guard < 64 {
            guard += 1;
            match n.goal {
                Goal::Hunt { ground, left, fighting } => {
                    let (gx, gz, rank) = self.grounds.get(ground).copied().unwrap_or((n.pos.x, n.pos.z, 0));
                    if !fighting {
                        let target = self.standable((gx, gz), active);
                        let before = (n.pos.x, n.pos.z);
                        let Some((used, arrived)) = self.walk(n, target, TRAVEL_SPEED, dt, active) else {
                            self.give_up(n);
                            continue;
                        };
                        n.fatigue = (n.fatigue + dist(before, (n.pos.x, n.pos.z)) / 9000.0).min(1.0);
                        dt -= used;
                        if arrived {
                            n.no_target = 0.0;
                            n.goal = Goal::Hunt { ground, left, fighting: true };
                        }
                    } else {
                        let used = dt.min(left);
                        if active {
                            // the world picks the mob (`chase`); here the NPC walks up to it
                            match n.chase {
                                Some(c) if dist((n.pos.x, n.pos.z), c) > n.stop => {
                                    let _ = self.walk(n, c, TRAVEL_SPEED, used, false);
                                }
                                Some(c) => {
                                    n.pos.speed = 0;
                                    n.pos.yaw = (n.pos.x - c.0).atan2(n.pos.z - c.1);
                                }
                                None => n.pos.speed = 0,
                            }
                        } else {
                            n.pos.speed = 0;
                            // fighting at a distance from everybody: about one landed blow every two seconds, and a blow taken every ten
                            let (skill, hit) = (weapon_skill(&n.group), 0.5 * used);
                            let g = skill_gain(n.skill(skill), rank, 0.12) * hit;
                            self.train(n, skill, g);
                            let g = skill_gain(n.skill("Health"), rank, 0.06) * 0.1 * used;
                            self.train(n, "Health", g);
                            n.fatigue = (n.fatigue + used / 400.0).min(1.0);
                        }
                        dt -= used;
                        if left - used <= 1e-3 {
                            self.finish_hunt(n);
                        } else {
                            n.goal = Goal::Hunt { ground, left: left - used, fighting: true };
                        }
                    }
                }
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
                        let mut t = (p.x, p.z);
                        for _ in 0..6 {
                            let a = frac(&mut n.rng) * 6.283;
                            let r = frac(&mut n.rng) * WANDER_R;
                            t = (p.x + a.cos() * r, p.z + a.sin() * r);
                            if self.nav.as_ref().map_or(true, |nav| !active || nav.walkable(t.0, t.1)) {
                                break;
                            }
                        }
                        t
                    });
                    let can = dt.min(left);
                    let target = self.standable(target, active);
                    let Some((used, arrived)) = self.walk(n, target, WALK_SPEED, can, active) else {
                        self.give_up(n);
                        continue;
                    };
                    dt -= used;
                    if left - used <= 1e-3 {
                        n.pos.speed = 0;
                        n.path.clear();
                        n.path_goal = None;
                        self.think(n);
                    } else {
                        n.goal = Goal::Wander { left: left - used, to: if arrived { None } else { Some(target) } };
                    }
                }
                Goal::Travel { dest } => {
                    let p = &self.places[dest];
                    let target = self.standable((p.x, p.z), active);
                    let before = (n.pos.x, n.pos.z);
                    let Some((used, arrived)) = self.walk(n, target, TRAVEL_SPEED, dt, active) else {
                        self.give_up(n);
                        continue;
                    };
                    n.fatigue = (n.fatigue + dist(before, (n.pos.x, n.pos.z)) / 9000.0).min(1.0);
                    dt -= used;
                    if arrived {
                        n.at = dest;
                        self.think(n);
                    }
                }
            }
        }
    }

    /// An NPC that becomes visible stands on walkable ground: if the abstract walk left it in water or inside a wall, move it to the nearest free cell.
    /// Returns false (and leaves it alone) when there is no free cell within 80 m.
    fn land(&self, n: &mut Npc) -> bool {
        n.path.clear();
        n.path_goal = None;
        let Some(nav) = &self.nav else { return true };
        match nav.snap((n.pos.x, n.pos.z), 40) {
            Some((x, z)) => {
                n.pos.x = x;
                n.pos.z = z;
                n.good = (x, z);
                true
            }
            None => false,
        }
    }

    /// Put an active NPC at the terrain's height and tell the clients so (they stop snapping it to their own ground).
    fn stand(&self, n: &mut Npc) {
        if let Some(nav) = &self.nav {
            // never end a tick in a wall or in water (a leg may graze a corner): step back to where it last stood and plan again
            if nav.walkable(n.pos.x, n.pos.z) {
                n.good = (n.pos.x, n.pos.z);
            } else {
                n.pos.x = n.good.0;
                n.pos.z = n.good.1;
                n.pos.speed = 0;
                n.path.clear();
                n.path_goal = None;
            }
        }
        if let Some(h) = self.nav.as_ref().and_then(|nav| nav.height(n.pos.x, n.pos.z)) {
            n.pos.y = h;
            n.pos.flags &= !FLAG_SNAP;
        }
    }

    /// One server tick: decide every NPC's LOD from the players' (wire x/z) positions and step those that are due.
    /// `radius` is the players' interest radius (Active). Returns how many NPCs are at each level.
    pub fn step(&mut self, dt: f32, players: &[(f32, f32)], radius: f32) -> LodCounts {
        self.tick = self.tick.wrapping_add(1);
        self.promoted.clear();
        self.revived.clear();
        let cell = NEAR_RADIUS;
        let mut grid: HashMap<(i32, i32), Vec<(f32, f32)>> = HashMap::new();
        for &(x, z) in players {
            grid.entry(((x / cell).floor() as i32, (z / cell).floor() as i32)).or_default().push((x, z));
        }
        let mut counts = LodCounts::default();
        let mut npcs = std::mem::take(&mut self.npcs);
        for (i, n) in npcs.iter_mut().enumerate() {
            if n.down > 0.0 {
                // lying dead (killed while somebody watched): nothing else happens until it is back at home
                n.down -= dt;
                if n.down <= 0.0 {
                    self.revive(n);
                    self.revived.push(n.id);
                }
                counts.active += 1;
                continue;
            }
            // a far NPC re-checks its distance only now and then; one that is already close checks every tick
            if n.lod == Lod::Far && (i as u32 + self.tick) % 15 != 0 {
                n.acc += dt;
                if n.acc >= FAR_EVERY {
                    let a = std::mem::take(&mut n.acc);
                    self.advance(n, a, false);
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
            let mut lod = if best <= radius { Lod::Active } else if best <= NEAR_RADIUS { Lod::Near } else { Lod::Far };
            n.acc += dt;
            if lod == Lod::Active && n.lod != Lod::Active {
                // Becoming visible: tried twice a second at most, and only from ground an avatar can stand on (the abstract walk of a near
                // or far NPC ignores lakes and mountains). Until then it stays near, unseen, and walks on.
                if (i as u32 + self.tick) % 8 != 0 {
                    lod = Lod::Near;
                } else {
                    // catch up to "now" before anybody sees it
                    let a = std::mem::take(&mut n.acc);
                    self.advance(n, a, false);
                    if !self.land(n) {
                        lod = Lod::Near;
                    }
                }
            }
            if lod == Lod::Active && n.lod != Lod::Active {
                self.promoted.push(n.id);
            }
            n.lod = lod;
            let every = match lod {
                Lod::Active => 0.0,
                Lod::Near => NEAR_EVERY,
                Lod::Far => FAR_EVERY,
            };
            if n.acc >= every {
                let a = std::mem::take(&mut n.acc);
                self.advance(n, a, lod == Lod::Active);
            }
            if lod == Lod::Active {
                self.stand(n);
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
                    Goal::Hunt { ground, left, .. } => (3, left, ground as i64),
                };
                NpcState { idx: (n.id - NPC_ID_BASE) as u32, name: n.info.name.clone(), race: n.info.race.clone(), gender: n.info.gender.clone(), gear: n.info.gear.clone(), skills: serde_json::to_string(&n.skills).unwrap_or_default(), weapon: n.info.weapon.clone(), home: n.home, at: n.at, goal_kind: kind, goal_a: a, goal_dest: dest, fatigue: n.fatigue, x: n.pos.x, z: n.pos.z, yaw: n.pos.yaw, rng: n.rng }
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
                3 if st.goal_dest >= 0 && (st.goal_dest as usize) < self.grounds.len() => Goal::Hunt { ground: st.goal_dest as usize, left: st.goal_a, fighting: false },
                _ => continue,
            };
            if st.home >= places || st.at >= places || !st.x.is_finite() || !st.z.is_finite() {
                continue;
            }
            n.info.name = st.name.clone();
            n.info.race = st.race.clone();
            n.info.gender = st.gender.clone();
            n.info.gear = st.gear.clone();
            if let Ok(skills) = serde_json::from_str::<BTreeMap<String, f32>>(&st.skills) {
                n.skills = skills;
            }
            n.max_hp = npc_hp(n.skill("Health"));
            n.hp = n.max_hp;
            if !st.weapon.is_empty() {
                n.info.weapon = st.weapon.clone();
            }
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
