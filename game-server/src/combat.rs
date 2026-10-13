//! Combat rules: weapon table, hit geometry, damage and projectiles.
//!
//! What comes from the game data (`game-viewer/data/weapons.json`, built by `tools/build_weapons.js` from `weaponpropertylist.xml`, the skill files and the animation
//! clips): each weapon's class, reach (m), arc angle (degrees), rank, and the length of every upper-body animation clip. The blow of a melee swing lands 45% into its
//! clip (the skills' `Synchronized_StateChange`), a bow takes as long as `bow_load` to draw, arrows fly like `m_Archery_Arrow_01` (100 m/s, 300 m range).
//!
//! What the game does not ship (damage numbers, hit points and the like are computed by its server-side Java, which is not decodable) is *invented* below and marked so:
//! `base_damage`, `class_mult`, the hit points and the mob damage tables. They are one place, so tuning is easy.
use std::collections::HashMap;

use serde::Deserialize;

/// The viewer scales the world so a player is 1.5x a human (js/collision.js PLAYER_SCALE): reaches from the data are in game metres.
pub const WORLD_SCALE: f32 = 1.5;
/// Radius of a body for hit tests (game metres, before WORLD_SCALE).
pub const BODY_RADIUS: f32 = 0.55;
/// Fraction of the swing clip at which the blow lands.
pub const HIT_AT: f32 = 0.45;
pub const PLAYER_HP: f32 = 120.0;
/// Arrows and bolts: gravity (the viewer uses 35% of g on arrows) and the vertical slack of the hit test (the server does not know the terrain: mobs report y = 0).
pub const ARROW_GRAVITY: f32 = 9.8 * 0.35;
pub const VERTICAL_SLACK: f32 = 14.0;

#[derive(Debug, Clone, Deserialize)]
pub struct WeaponDef {
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub reach: Option<f32>,
    #[serde(default)]
    pub angle: Option<f32>,
    #[serde(default)]
    pub rank: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct WeaponFile {
    weapons: HashMap<String, WeaponDef>,
    #[serde(rename = "clipDur", default)]
    clip_dur: HashMap<String, f32>,
}

#[derive(Debug, Default)]
pub struct Weapons {
    pub defs: HashMap<String, WeaponDef>,
    clip_dur: HashMap<String, f32>,
}

impl Weapons {
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        Self::parse(&text).map_err(|e| format!("{path}: {e}"))
    }
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        let f: WeaponFile = serde_json::from_str(text)?;
        Ok(Weapons { defs: f.weapons, clip_dur: f.clip_dur })
    }
    pub fn get(&self, id: &str) -> Option<&WeaponDef> {
        self.defs.get(id)
    }
    /// Length (seconds) of a clip; `or` if the data doesn't have it.
    pub fn clip(&self, name: &str, or: f32) -> f32 {
        self.clip_dur.get(name).copied().unwrap_or(or)
    }
}

/// How a class attacks: the upper-body clip family (`singlehand_attack_01`, `2handed_attack_01`, ...) decides the swing duration.
pub fn swing_clip(group: &str) -> &'static str {
    match group {
        "greatsword" | "greataxe" | "greatclub" => "2handed_attack_01",
        "polearm" => "halberd_attack_01",
        "knife" => "2daggers_attack_01",
        "staff" => "magic_fire_01",
        "thrown" => "attack_throw",
        "bow" => "bow_fire",
        _ => "singlehand_attack_01",
    }
}

pub fn is_melee(group: &str) -> bool {
    !matches!(group, "bow" | "staff" | "thrown" | "shield")
}

/// INVENTED (the game's damage table is server-side): damage of a hit before the class multiplier, from the weapon rank (10..80 in the data).
pub fn base_damage(rank: f32) -> f32 {
    6.0 + rank * 0.55
}

/// INVENTED: heavier classes hit harder (two-handers 1.6x, daggers 0.7x ...).
pub fn class_mult(group: &str) -> f32 {
    match group {
        "knife" => 0.7,
        "sword" | "sithra" => 1.0,
        "club" => 1.05,
        "axe" => 1.1,
        "polearm" => 1.15,
        "greatsword" => 1.6,
        "greatclub" => 1.65,
        "greataxe" => 1.7,
        "bow" => 1.3,
        "staff" => 1.2,
        "thrown" => 0.9,
        _ => 1.0,
    }
}

/// Damage type name used for the impact effect / sound: "slashing", "bludgeoning", "piercing", "arrow", "fire" (matches `se_impact_*` in the effect data).
pub fn damage_type(group: &str) -> &'static str {
    match group {
        "club" | "greatclub" => "bludgeoning",
        "knife" => "piercing",
        "bow" => "arrow",
        "staff" => "fire",
        _ => "slashing",
    }
}

/// INVENTED hit points of a mob by the difficulty label of its spawn.
pub fn mob_hp(difficulty: &str) -> f32 {
    match difficulty {
        "Very Easy" => 25.0,
        "Easy" => 60.0,
        "Medium" => 120.0,
        "Hard" => 250.0,
        "Very Hard" => 500.0,
        "Champion" => 1500.0,
        "Boss" => 4000.0,
        _ => 100.0,
    }
}

/// 0 (Very Easy) .. 6 (Boss): how dangerous a mob is, for NPCs choosing what to hunt.
pub fn difficulty_rank(difficulty: &str) -> u8 {
    match difficulty {
        "Very Easy" => 0,
        "Easy" => 1,
        "Medium" => 2,
        "Hard" => 3,
        "Very Hard" => 4,
        "Champion" => 5,
        "Boss" => 6,
        _ => 2,
    }
}

/// The game's skill (from its skill list) that a weapon class trains and is limited by.
pub fn weapon_skill(group: &str) -> &'static str {
    match group {
        "axe" => "Axes",
        "club" => "Clubs and Maces",
        "greatsword" => "Greatswords",
        "greataxe" => "Greataxes",
        "greatclub" => "Greatclubs",
        "knife" => "Knives",
        "polearm" => "Polearms",
        "bow" => "Archery",
        _ => "Swords",
    }
}

/// INVENTED: an NPC's hit points from its Health skill (0..100); a player has `PLAYER_HP`.
pub fn npc_hp(health: f32) -> f32 {
    80.0 + 1.2 * health
}

/// INVENTED: how a weapon skill (0..100) scales the damage of a blow (0.6x unskilled, 1.4x grandmaster).
pub fn skill_damage(skill: f32) -> f32 {
    0.6 + 0.008 * skill.clamp(0.0, 100.0)
}

/// INVENTED: the weapon skill a mob of this rank is a fair fight for. Training from it dries up once the skill is well past it.
pub fn mob_skill(rank: u8) -> f32 {
    [10.0, 25.0, 45.0, 65.0, 85.0, 100.0, 100.0][rank.min(6) as usize]
}

/// INVENTED damage of one mob blow.
pub fn mob_damage(difficulty: &str) -> f32 {
    match difficulty {
        "Very Easy" => 4.0,
        "Easy" => 8.0,
        "Medium" => 14.0,
        "Hard" => 22.0,
        "Very Hard" => 35.0,
        "Champion" => 60.0,
        "Boss" => 90.0,
        _ => 12.0,
    }
}

/// Damage of one blow of weapon `w` with a +-10% roll in `roll` (0..1); `power` scales ranged weapons (draw).
pub fn damage(w: &WeaponDef, group: &str, power: f32, roll: f32) -> f32 {
    let rank = w.rank.unwrap_or(30.0);
    (base_damage(rank) * class_mult(group) * (0.9 + 0.2 * roll) * power.clamp(0.2, 1.0)).round().max(1.0)
}

/// Unit forward vector (x, z) of a wire yaw: the camera looks along -Z rotated by yaw (three.js).
pub fn forward(yaw: f32) -> (f32, f32) {
    (-yaw.sin(), -yaw.cos())
}

/// Does a target body (radius `tr`) at (tx, tz) lie inside the swing of an attacker at (ax, az) facing `yaw`? `reach` and `arc_deg` are weapon values (metres, degrees: the
/// full width of the arc); the body's own width widens the cone.
pub fn in_arc(a: (f32, f32), yaw: f32, t: (f32, f32), reach: f32, arc_deg: f32, tr: f32) -> bool {
    let (dx, dz) = (t.0 - a.0, t.1 - a.1);
    let d = (dx * dx + dz * dz).sqrt();
    if d > reach + tr {
        return false;
    }
    if d < 0.05 {
        return true;
    }
    let (fx, fz) = forward(yaw);
    let cos = ((dx * fx + dz * fz) / d).clamp(-1.0, 1.0);
    cos.acos() <= (arc_deg.to_radians() * 0.5).max(0.35) + (tr / d).atan()
}

/// A blocking player takes 25% of a blow that comes from inside this half-angle in front of him.
pub const BLOCK_HALF: f32 = 1.0;
pub const BLOCK_FACTOR: f32 = 0.25;
pub fn blocks(target_yaw: f32, target: (f32, f32), from: (f32, f32)) -> bool {
    let (dx, dz) = (from.0 - target.0, from.1 - target.1);
    let d = (dx * dx + dz * dz).sqrt();
    if d < 0.05 {
        return true;
    }
    let (fx, fz) = forward(target_yaw);
    ((dx * fx + dz * fz) / d).clamp(-1.0, 1.0).acos() <= BLOCK_HALF
}

/// An arrow or spell in flight.
#[derive(Debug, Clone)]
pub struct Projectile {
    pub owner: u16,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub left: f32,
    pub group: String,
    pub dmg: f32,
    pub spell: bool,
}

impl Projectile {
    /// Advance by dt; returns the segment travelled (from, to).
    pub fn step(&mut self, dt: f32) -> ([f32; 3], [f32; 3]) {
        let from = self.pos;
        if !self.spell {
            self.vel[1] -= ARROW_GRAVITY * dt;
        }
        for i in 0..3 {
            self.pos[i] += self.vel[i] * dt;
        }
        self.left -= (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1] + self.vel[2] * self.vel[2]).sqrt() * dt;
        (from, self.pos)
    }
}

/// Does the segment a->b pass within `r` (horizontally) of the target at (tx, tz), at a height within VERTICAL_SLACK of `ref_y`?
pub fn segment_hits(a: [f32; 3], b: [f32; 3], t: (f32, f32), r: f32, ref_y: f32) -> bool {
    let (sx, sz) = (b[0] - a[0], b[2] - a[2]);
    let len2 = sx * sx + sz * sz;
    let k = if len2 < 1e-6 { 0.0 } else { (((t.0 - a[0]) * sx + (t.1 - a[2]) * sz) / len2).clamp(0.0, 1.0) };
    let (px, pz, py) = (a[0] + sx * k, a[2] + sz * k, a[1] + (b[1] - a[1]) * k);
    let (dx, dz) = (px - t.0, pz - t.1);
    dx * dx + dz * dz <= r * r && (py - ref_y).abs() <= VERTICAL_SLACK
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"{"weapons":{"r_Test Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":30},
        "r_Test Axe":{"group":"greataxe","kind":"melee","reach":2.45,"angle":65,"rank":40}},"clipDur":{"singlehand_attack_01":1.0,"2handed_attack_01":1.167}}"#;

    #[test]
    fn weapon_table_parses() {
        let w = Weapons::parse(DATA).unwrap();
        assert_eq!(w.get("r_Test Sword").unwrap().reach, Some(2.1));
        assert_eq!(w.clip(swing_clip("greataxe"), 1.0), 1.167);
        assert_eq!(w.clip("nope", 0.5), 0.5);
    }

    #[test]
    fn damage_scales_with_rank_and_class() {
        let w = Weapons::parse(DATA).unwrap();
        let sword = damage(w.get("r_Test Sword").unwrap(), "sword", 1.0, 0.5);
        let axe = damage(w.get("r_Test Axe").unwrap(), "greataxe", 1.0, 0.5);
        assert!(axe > sword * 1.8, "{axe} vs {sword}");
        assert!(damage(w.get("r_Test Sword").unwrap(), "bow", 0.2, 0.5) < damage(w.get("r_Test Sword").unwrap(), "bow", 1.0, 0.5));
    }

    #[test]
    fn arc_requires_reach_and_facing() {
        let r = 2.1 * WORLD_SCALE;
        let tr = BODY_RADIUS * WORLD_SCALE;
        // yaw 0 looks along -z
        assert!(in_arc((0.0, 0.0), 0.0, (0.0, -2.5), r, 60.0, tr));
        assert!(!in_arc((0.0, 0.0), 0.0, (0.0, -9.0), r, 60.0, tr), "too far");
        assert!(!in_arc((0.0, 0.0), 0.0, (0.0, 2.5), r, 60.0, tr), "behind");
        assert!(!in_arc((0.0, 0.0), 0.0, (3.0, 0.0), r, 60.0, tr), "to the side");
        // yaw pi/2 looks along -x
        assert!(in_arc((0.0, 0.0), std::f32::consts::FRAC_PI_2, (-2.5, 0.0), r, 60.0, tr));
    }

    #[test]
    fn block_covers_the_front_only() {
        assert!(blocks(0.0, (0.0, 0.0), (0.0, -3.0)));
        assert!(!blocks(0.0, (0.0, 0.0), (0.0, 3.0)));
    }

    #[test]
    fn arrows_fly_and_hit_a_target_in_line() {
        let mut p = Projectile { owner: 1, pos: [0.0, 1.6, 0.0], vel: [0.0, 0.0, -100.0], left: 300.0, group: "bow".into(), dmg: 30.0, spell: false };
        let mut hit = false;
        for _ in 0..30 {
            let (a, b) = p.step(1.0 / 15.0);
            if segment_hits(a, b, (0.3, -40.0), 0.8, 0.0) {
                hit = true;
                break;
            }
        }
        assert!(hit);
        assert!(p.vel[1] < 0.0, "gravity pulls the arrow down");
        assert!(!segment_hits([0.0, 1.6, 0.0], [0.0, 1.6, -10.0], (5.0, -5.0), 0.8, 0.0));
    }
}
