//! Server-owned mobs. Spawn points come from `game-viewer/data/mobspawns.json` (built by `tools/build_mobspawns.js`); each spawn
//! holds one or more mob types with a quantity. Mobs idle and wander inside a small leash around their spawn point — there is no
//! combat, aggro or respawn yet. Positions use the wire convention (viewer three.js space: game z negated).
use std::collections::HashMap;

use serde::Deserialize;

use crate::proto::{MobType, PosUpdate, FLAG_SNAP};

/// Mobs wander at most this far (metres) from their spawn point.
pub const LEASH: f32 = 14.0;
const WALK_SPEED: f32 = 1.3; // m/s
const IDLE_SECS: (f32, f32) = (2.0, 9.0);

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
    home: (f32, f32),
    target: Option<(f32, f32)>,
    wait: f32,
    rng: u32,
}

pub struct Mobs {
    pub mobs: Vec<Mob>,
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

    fn step(&mut self, dt: f32) {
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
}

impl Mobs {
    pub fn empty() -> Self {
        Mobs { mobs: Vec::new(), types: Vec::new(), grid: HashMap::new(), cell: 1.0 }
    }

    pub fn load(path: &str, radius: f32) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let spawns: Vec<SpawnFile> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        Ok(Self::from_spawns(spawns, radius))
    }

    fn from_spawns(spawns: Vec<SpawnFile>, radius: f32) -> Self {
        let mut me = Mobs { cell: (radius + LEASH).max(1.0), ..Self::empty() };
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
                        home,
                        target: None,
                        wait: 0.0,
                        rng: (s.id.wrapping_mul(2654435761) ^ (id as u32 + 1).wrapping_mul(40503) ^ k).max(1),
                    };
                    let (a, r) = (mob.next() * std::f32::consts::TAU, mob.next().sqrt() * LEASH * 0.5);
                    mob.pos.x += a.cos() * r;
                    mob.pos.z += a.sin() * r;
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

    pub fn step(&mut self, dt: f32) {
        for m in &mut self.mobs {
            m.step(dt);
        }
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
            m.step(1.0 / 15.0);
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
