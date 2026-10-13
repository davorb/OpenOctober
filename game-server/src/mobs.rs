//! Server-owned mobs. Spawn points come from `game-viewer/data/mobspawns.json` (built by `tools/build_mobspawns.js`); each spawn
//! holds one or more mob types with a quantity. Mobs idle and wander inside a small leash around their spawn point; every mob except the 'Very Easy' ones
//! attacks a player who comes within AGGRO metres, chases him up to CHASE_LEASH from home, hits him every ATTACK_EVERY seconds while in reach and walks home (healed)
//! when he escapes. Killed mobs lie there as FLAG_DEAD for a while and come back at full health RESPAWN seconds later. Hit points and damage per difficulty are the
//! invented tables of `combat.rs`. Positions use the wire convention (viewer three.js space: game z negated).
use std::collections::HashMap;

use serde::Deserialize;

use crate::combat::{difficulty_rank, mob_damage, mob_hp, WORLD_SCALE};
use crate::proto::{MobType, PosUpdate, FLAG_DEAD, FLAG_FIGHT, FLAG_SNAP};

/// Mobs wander at most this far (metres) from their spawn point.
pub const LEASH: f32 = 14.0;
const WALK_SPEED: f32 = 1.3; // m/s
const IDLE_SECS: (f32, f32) = (2.0, 9.0);
/// A mob notices a player this close (m), chases at CHASE_SPEED and gives up when it is CHASE_LEASH from home.
pub const AGGRO: f32 = 28.0;
const CHASE_SPEED: f32 = 4.6;
pub const CHASE_LEASH: f32 = 90.0;
/// Reach of a mob blow (m, world space: bodies are WORLD_SCALE bigger than in the data) and the time between blows.
pub const ATTACK_RANGE: f32 = 2.2 * WORLD_SCALE;
const ATTACK_EVERY: f32 = 2.0;
pub const RESPAWN: f32 = 30.0;
/// Mobs that were hit stop caring about a player after this long without reaching him.
const SCAN_EVERY: u32 = 8;

#[derive(Debug, Deserialize)]
struct SpawnFile {
    id: u32,
    x: f32,
    z: f32,
    mobs: Vec<SpawnMob>,
}

#[derive(Debug, Deserialize)]
struct SpawnMob {
    id: u16,
    name: String,
    qty: u32,
    difficulty: String,
}

pub struct Mob {
    pub id: u16,
    pub kind: u16,
    pub pos: PosUpdate,
    pub hp: f32,
    pub max_hp: f32,
    pub alive: bool,
    /// `combat::difficulty_rank` of the spawn's difficulty.
    pub rank: u8,
    dmg: f32,
    aggressive: bool,
    /// Player being fought, and seconds until the next blow / until the respawn.
    aggro: Option<u16>,
    cd: f32,
    respawn: f32,
    spawn: (f32, f32),
    home: (f32, f32),
    target: Option<(f32, f32)>,
    wait: f32,
    rng: u32,
}

/// A mob blow on a player (the world applies it: blocking, death, messages).
#[derive(Debug, Clone, Copy)]
pub struct MobAttack {
    pub mob: u16,
    pub player: u16,
    pub dmg: f32,
}

pub struct Mobs {
    pub mobs: Vec<Mob>,
    ticks: u32,
    pub types: Vec<MobType>,
    /// Home cell -> mob indices. Mobs never leave their leash, so a 3x3 lookup with cells >= radius + leash never misses one.
    grid: HashMap<(i32, i32), Vec<u32>>,
    cell: f32,
}

impl Mob {
    fn next(&mut self) -> f32 {
        // xorshift32 -> [0, 1)
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Wander (the original behaviour), used when no player is being fought.
    fn wander(&mut self, dt: f32) {
        self.pos.flags &= !FLAG_FIGHT;
        match self.target {
            None => {
                self.pos.speed = 0;
                self.wait -= dt;
                if self.wait <= 0.0 {
                    let (a, r) = (self.next() * std::f32::consts::TAU, self.next().sqrt() * LEASH);
                    self.target = Some((self.home.0 + a.cos() * r, self.home.1 + a.sin() * r));
                }
            }
            Some((tx, tz)) => {
                let (dx, dz) = (tx - self.pos.x, tz - self.pos.z);
                let d = (dx * dx + dz * dz).sqrt();
                let stride = WALK_SPEED * dt;
                if d <= stride {
                    self.pos.x = tx;
                    self.pos.z = tz;
                    self.target = None;
                    self.pos.speed = 0;
                    self.wait = IDLE_SECS.0 + self.next() * (IDLE_SECS.1 - IDLE_SECS.0);
                } else {
                    self.pos.x += dx / d * stride;
                    self.pos.z += dz / d * stride;
                    self.pos.yaw = dx.atan2(dz);
                    self.pos.speed = (WALK_SPEED * 10.0) as u8;
                }
            }
        }
    }

    /// One tick. `players` = (id, x, z) of the living players; returns the blow this mob lands, if any.
    fn step(&mut self, dt: f32, tick: u32, players: &[(u16, f32, f32)]) -> Option<MobAttack> {
        if !self.alive {
            self.pos.speed = 0;
            self.respawn -= dt;
            if self.respawn <= 0.0 {
                self.alive = true;
                self.hp = self.max_hp;
                self.pos.flags = FLAG_SNAP;
                self.pos.x = self.spawn.0;
                self.pos.z = self.spawn.1;
                self.target = None;
                self.aggro = None;
            }
            return None;
        }
        // notice a player (time-sliced: a mob looks around every SCAN_EVERY ticks)
        if self.aggro.is_none() && self.aggressive && (tick.wrapping_add(self.id as u32)) % SCAN_EVERY == 0 {
            let mut best = (AGGRO * AGGRO, None);
            for &(id, x, z) in players {
                let (dx, dz) = (x - self.pos.x, z - self.pos.z);
                let d2 = dx * dx + dz * dz;
                if d2 < best.0 && ((x - self.home.0).powi(2) + (z - self.home.1).powi(2)).sqrt() < CHASE_LEASH {
                    best = (d2, Some(id));
                }
            }
            self.aggro = best.1;
        }
        let Some(pid) = self.aggro else {
            if self.hp < self.max_hp {
                self.hp = (self.hp + self.max_hp * 0.05 * dt).min(self.max_hp); // regenerates once the fight is over
            }
            self.wander(dt);
            return None;
        };
        let Some(&(_, px, pz)) = players.iter().find(|p| p.0 == pid) else {
            self.aggro = None; // he died or left
            return None;
        };
        let (dx, dz) = (px - self.pos.x, pz - self.pos.z);
        let d = (dx * dx + dz * dz).sqrt();
        let home_d = ((self.pos.x - self.home.0).powi(2) + (self.pos.z - self.home.1).powi(2)).sqrt();
        if home_d > CHASE_LEASH {
            // he got away: walk home, healed
            self.aggro = None;
            self.hp = self.max_hp;
            self.target = Some(self.home);
            return None;
        }
        self.pos.flags |= FLAG_FIGHT;
        self.pos.yaw = dx.atan2(dz);
        if d > ATTACK_RANGE * 0.9 {
            let stride = (CHASE_SPEED * dt).min(d - ATTACK_RANGE * 0.8);
            self.pos.x += dx / d * stride;
            self.pos.z += dz / d * stride;
            self.pos.speed = (CHASE_SPEED * 10.0) as u8;
            self.cd = self.cd.max(0.4);
            return None;
        }
        self.pos.speed = 0;
        self.cd -= dt;
        if self.cd <= 0.0 {
            self.cd = ATTACK_EVERY;
            return Some(MobAttack { mob: self.id, player: pid, dmg: self.dmg });
        }
        None
    }
}

impl Mobs {
    pub fn empty() -> Self {
        Mobs { mobs: Vec::new(), ticks: 0, types: Vec::new(), grid: HashMap::new(), cell: 1.0 }
    }

    pub fn load(path: &str, radius: f32) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let spawns: Vec<SpawnFile> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        Ok(Self::from_spawns(spawns, radius))
    }

    fn from_spawns(spawns: Vec<SpawnFile>, radius: f32) -> Self {
        let mut me = Mobs { cell: (radius + CHASE_LEASH).max(1.0), ..Self::empty() };
        let mut types: HashMap<u16, MobType> = HashMap::new();
        'outer: for s in spawns {
            for m in &s.mobs {
                types.entry(m.id).or_insert_with(|| MobType { id: m.id, name: m.name.clone(), difficulty: m.difficulty.clone() });
                for k in 0..m.qty {
                    if me.mobs.len() >= u16::MAX as usize {
                        eprintln!("mobs: more than {} mobs, ignoring the rest", u16::MAX);
                        break 'outer;
                    }
                    let id = me.mobs.len() as u16;
                    // wire space negates game z; scatter a spawn's mobs around its point so they don't stack
                    let home = (s.x, -s.z);
                    let mut mob = Mob {
                        id,
                        kind: m.id,
                        pos: PosUpdate { x: home.0, y: 0.0, z: home.1, yaw: 0.0, speed: 0, flags: FLAG_SNAP },
                        hp: mob_hp(&m.difficulty),
                        max_hp: mob_hp(&m.difficulty),
                        alive: true,
                        rank: difficulty_rank(&m.difficulty),
                        dmg: mob_damage(&m.difficulty),
                        aggressive: m.difficulty != "Very Easy",
                        aggro: None,
                        cd: 0.5,
                        respawn: 0.0,
                        spawn: home,
                        home,
                        target: None,
                        wait: 0.0,
                        rng: (s.id.wrapping_mul(2654435761) ^ (id as u32 + 1).wrapping_mul(40503) ^ k).max(1),
                    };
                    let (a, r) = (mob.next() * std::f32::consts::TAU, mob.next().sqrt() * LEASH * 0.5);
                    mob.pos.x += a.cos() * r;
                    mob.pos.z += a.sin() * r;
                    mob.spawn = (mob.pos.x, mob.pos.z);
                    mob.wait = mob.next() * IDLE_SECS.1;
                    me.mobs.push(mob);
                }
            }
        }
        for (i, m) in me.mobs.iter().enumerate() {
            me.grid.entry(me.key(m.home.0, m.home.1)).or_default().push(i as u32);
        }
        me.types = types.into_values().collect();
        me.types.sort_by_key(|t| t.id);
        me
    }

    fn key(&self, x: f32, z: f32) -> (i32, i32) {
        ((x / self.cell).floor() as i32, (z / self.cell).floor() as i32)
    }

    /// Advance every mob; returns the blows landed on players this tick. `players` = (id, x, z) of the living ones.
    pub fn step(&mut self, dt: f32, players: &[(u16, f32, f32)]) -> Vec<MobAttack> {
        self.ticks = self.ticks.wrapping_add(1);
        let tick = self.ticks;
        self.mobs.iter_mut().filter_map(|m| m.step(dt, tick, players)).collect()
    }

    /// A player's blow on mob `id`: returns (hp left, killed). The mob turns on the attacker.
    pub fn damage(&mut self, id: u16, dmg: f32, by: u16) -> Option<(f32, bool)> {
        let m = self.mobs.get_mut(id as usize)?;
        if !m.alive {
            return None;
        }
        m.hp = (m.hp - dmg).max(0.0);
        m.aggro = Some(by);
        if m.hp <= 0.0 {
            m.alive = false;
            m.aggro = None;
            m.respawn = RESPAWN;
            m.pos.speed = 0;
            m.pos.flags = FLAG_SNAP | FLAG_DEAD;
            return Some((0.0, true));
        }
        Some((m.hp, false))
    }

    /// One hunting ground per spawn point: (x, z, rank of its toughest mob).
    pub fn grounds(&self) -> Vec<(f32, f32, u8)> {
        let mut by_home: HashMap<(i32, i32), (f32, f32, u8)> = HashMap::new();
        for m in &self.mobs {
            let e = by_home.entry((m.home.0.round() as i32, m.home.1.round() as i32)).or_insert((m.home.0, m.home.1, 0));
            e.2 = e.2.max(m.rank);
        }
        let mut v: Vec<_> = by_home.into_values().collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        v
    }

    /// Indices of mobs that may be within the interest radius of (x, z); the caller does the exact distance test.
    pub fn candidates(&self, x: f32, z: f32) -> impl Iterator<Item = u32> + '_ {
        let (cx, cz) = self.key(x, z);
        (-1..=1).flat_map(move |dx| (-1..=1).map(move |dz| (cx + dx, cz + dz))).filter_map(|k| self.grid.get(&k)).flatten().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Mobs {
        let spawns: Vec<SpawnFile> = serde_json::from_str(
            r#"[{"id":1,"x":100,"z":200,"mobs":[{"id":7,"name":"Troll","qty":3,"difficulty":"Hard"}]},
                {"id":2,"x":5000,"z":-300,"mobs":[{"id":9,"name":"Wolf","qty":1,"difficulty":"Easy"},{"id":7,"name":"Troll","qty":1,"difficulty":"Hard"}]}]"#,
        )
        .unwrap();
        Mobs::from_spawns(spawns, 900.0)
    }

    #[test]
    fn expands_quantities_and_negates_z() {
        let m = sample();
        assert_eq!(m.mobs.len(), 5);
        assert_eq!(m.types.iter().map(|t| t.id).collect::<Vec<_>>(), vec![7, 9]);
        assert!(m.mobs[0].pos.z < 0.0 && (m.mobs[0].pos.z + 200.0).abs() <= LEASH);
        assert!(m.mobs[3].pos.z > 0.0);
    }

    #[test]
    fn wander_stays_on_the_leash() {
        let mut m = sample();
        for _ in 0..15 * 600 {
            m.step(1.0 / 15.0, &[]);
        }
        for mob in &m.mobs {
            let (dx, dz) = (mob.pos.x - mob.home.0, mob.pos.z - mob.home.1);
            assert!((dx * dx + dz * dz).sqrt() <= LEASH + 0.01);
        }
        assert!(m.mobs.iter().any(|mob| mob.pos.speed > 0 || mob.target.is_none()));
    }

    #[test]
    fn candidates_find_nearby_homes_only() {
        let m = sample();
        assert_eq!(m.candidates(150.0, -250.0).count(), 3);
        assert_eq!(m.candidates(5000.0, 300.0).count(), 2);
        assert_eq!(m.candidates(-9000.0, 9000.0).count(), 0);
    }
}
