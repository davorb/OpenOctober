//! Wire protocol.
//!
//! Control traffic is JSON text frames (`{"t": "...", ...}`); high-rate traffic is little-endian binary frames:
//!
//! * client -> server  `OP_POS`       `[1][x f32][y f32][z f32][yaw f32][speed u8][flags u8]`            (19 bytes)
//! * server -> client  `OP_SNAPSHOT`  `[2][tick u32][count u16]` then `count` x `[id u16][x][y][z][yaw][speed][flags]`
//! * server -> client  `OP_MOBS`      `[3][tick u32][count u16]` then `count` x `[id u16][type u16][x][y][z][yaw][speed][flags]`
//!   (mob ids are their own id space; `type` indexes the `mobtypes` roster; y is 0 with `FLAG_SNAP`, i.e. snap to the terrain)
//!
//! Coordinates are the viewer's three.js world space (game x, height y, *negated* game z). The server never interprets
//! them except for the x/z interest filter. `speed` is metres/second * 10 (clamped to 255).
use serde::{Deserialize, Serialize};

pub const OP_POS: u8 = 1;
pub const OP_SNAPSHOT: u8 = 2;
pub const OP_MOBS: u8 = 3;

pub const FLAG_SWIM: u8 = 1;
pub const FLAG_AIR: u8 = 2;
/// The sender has no real ground height (bots): receivers should snap the avatar to the terrain.
pub const FLAG_SNAP: u8 = 4;

pub const RACES: [&str; 6] = ["hum", "alf", "dwf", "ork", "mir", "mhm"];
pub const GENDERS: [&str; 2] = ["m", "f"];
pub const MAX_NAME: usize = 16;
pub const MAX_CHAT: usize = 200;
pub const MAX_GEAR: usize = 24;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PosUpdate {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub speed: u8,
    pub flags: u8,
}

impl PosUpdate {
    /// Bytes of a position without the opcode / id prefix.
    pub const BODY_LEN: usize = 18;

    pub fn write_body(&self, out: &mut Vec<u8>) {
        for v in [self.x, self.y, self.z, self.yaw] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(self.speed);
        out.push(self.flags);
    }

    pub fn read_body(b: &[u8]) -> Option<Self> {
        if b.len() < Self::BODY_LEN {
            return None;
        }
        let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let p = PosUpdate { x: f(0), y: f(4), z: f(8), yaw: f(12), speed: b[16], flags: b[17] };
        let sane = |v: f32| v.is_finite() && v.abs() < 1.0e6;
        (sane(p.x) && sane(p.y) && sane(p.z) && p.yaw.is_finite()).then_some(p)
    }

    /// Client -> server frame.
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + Self::BODY_LEN);
        v.push(OP_POS);
        self.write_body(&mut v);
        v
    }

    pub fn decode_frame(b: &[u8]) -> Option<Self> {
        match b.split_first() {
            Some((&OP_POS, rest)) => Self::read_body(rest),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerInfo {
    pub id: u16,
    pub name: String,
    pub race: String,
    pub gender: String,
    /// Armour set name from the game data ("" = none). Opaque to the server; clients ignore names they can't resolve.
    pub gear: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobType {
    pub id: u16,
    pub name: String,
    pub difficulty: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ClientMsg {
    Join { name: String, race: String, gender: String, #[serde(default)] gear: String },
    Chat { text: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ServerMsg<'a> {
    /// Sent once after a successful join: your id, the interest radius, and everybody already online (incl. you).
    Welcome { id: u16, radius: f32, tick_hz: u32, players: Vec<&'a PlayerInfo> },
    /// Sent once after `Welcome` when the server runs mobs: the roster of mob types referenced by `OP_MOBS` frames.
    Mobtypes { types: &'a [MobType] },
    Joined { player: &'a PlayerInfo },
    Left { id: u16 },
    Chat { id: u16, name: &'a str, text: &'a str },
    Error { msg: &'a str },
}

/// Trim, drop control characters, cap the length (in chars).
pub fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}
