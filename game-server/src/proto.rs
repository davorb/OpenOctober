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
/// Players only: the free-fly camera is on (not a jump). Such a player is out of the fight: mobs ignore him and his blows land nowhere.
pub const FLAG_FLY: u8 = 32;
/// Mobs only: the mob is dead (clients lay it down, then it disappears until it respawns).
pub const FLAG_DEAD: u8 = 8;
/// Mobs only: the mob is in combat (chasing / fighting a player).
pub const FLAG_FIGHT: u8 = 16;
/// Players only: the avatar is a server-run NPC (its `PlayerInfo::npc` is set too).
pub const FLAG_NPC: u8 = 64;

pub const RACES: [&str; 6] = ["hum", "alf", "dwf", "ork", "mir", "mhm"];
pub const GENDERS: [&str; 2] = ["m", "f"];
pub const MAX_NAME: usize = 16;
pub const MAX_CHAT: usize = 200;
pub const MAX_GEAR: usize = 24;
pub const MAX_WEAPON: usize = 48;

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
    /// Wielded weapon id from `data/weapons.json` ("r_Talon Axe"; "" = none) and whether it is drawn.
    pub weapon: String,
    pub drawn: bool,
    /// A server-run NPC, not a connected player: clients label it.
    pub npc: bool,
    /// What an NPC does for a living when it is not an adventurer: "vendor" (stands at a post and trades) or "trader" (carries goods between towns); "" otherwise.
    pub role: String,
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
    /// Wield a weapon (or sheathe / draw the one wielded).
    Equip { w: String, #[serde(default)] drawn: bool },
    /// A melee swing / cast / throw starts: the blow lands at 45% of its clip. `clip` / `dur` are for other players' animation only.
    Attack { #[serde(default)] kind: String, #[serde(default)] clip: String },
    Block { on: bool },
    /// Started drawing a bow (other players play the draw clip).
    Bow { #[serde(default)] what: String },
    /// An arrow or bolt leaves the weapon.
    Shoot { dir: [f32; 3], from: [f32; 3], #[serde(default)] power: f32, #[serde(default)] spell: bool },
    /// After dying: ask to get up (the server also does it by itself after a few seconds).
    Respawn,
    /// Take everything from a body's tombstone (`cid` of its `Corpse` message): you have to stand within a few metres of it.
    Loot { cid: u32 },
    /// Open the shop of the vendor NPC `npc` (you have to be within a few metres): answered by `Shop`.
    Shop { npc: u16 },
    /// Buy / sell up to `qty` units of `item` at that vendor: answered by `Trade`, `Stock` and a fresh `Shop`.
    Buy { npc: u16, item: String, #[serde(default = "one")] qty: u32 },
    Sell { npc: u16, item: String, #[serde(default = "one")] qty: u32 },
}

fn one() -> u32 {
    1
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
    Equip { id: u16, w: &'a str, drawn: bool },
    Swing { id: u16, kind: &'a str, clip: &'a str, group: &'a str },
    Block { id: u16, on: bool },
    Bow { id: u16, what: &'a str },
    Shoot { id: u16, from: [f32; 3], dir: [f32; 3], power: f32, spell: bool },
    /// A blow landed. `a` attacker (a player id, or a mob id when `am`), `v` victim kind ("p" / "m") and id `i`; `at` is the victim's position (mobs have no height: `snap`).
    Hit { a: u16, am: bool, v: &'a str, i: u16, dmg: f32, hp: f32, max: f32, grp: &'a str, blocked: bool, at: [f32; 3], snap: bool, killed: bool },
    Died { v: &'a str, i: u16 },
    /// A player got up again (full health) - the client puts itself back at a spawn point.
    Respawn { id: u16, hp: f32, max: f32 },
    /// A player died here: his body stays on the ground (CORPSE_SECS) for everybody, joiners get the existing ones too. `snap`: the body has no height of its own (bots).
    /// The body lies there for `Config::body_secs` (45 s); nothing can be taken from it. Then it is replaced by a `Tombstone` of the same `cid`
    /// (if it carried anything) or goes (`CorpseGone`).
    Corpse { cid: u32, name: &'a str, race: &'a str, gender: &'a str, gear: &'a str, x: f32, y: f32, z: f32, yaw: f32, snap: bool },
    /// The body of `cid` has turned into a tombstone (model `tombstone_player_characters_01`) with `gold` and `units` goods on it, for
    /// `Config::tomb_secs` (an hour) or until it is emptied. Late joiners get the tombstone without the body.
    Tombstone { cid: u32, name: &'a str, x: f32, y: f32, z: f32, yaw: f32, snap: bool, gold: f32, units: u32 },
    CorpseGone { cid: u32 },
    /// The tombstone `cid` has been emptied (it is gone).
    Looted { cid: u32 },
    /// You started taking everything from the tombstone `cid`: stand still for `secs`, damage or walking away interrupts.
    Looting { cid: u32, secs: f32 },
    /// The channel ended: `ok` = you got it; otherwise interrupted or somebody else was first.
    LootEnd { cid: u32, ok: bool },
    /// What you carry (sent after joining and whenever it changes): gold and goods by item id.
    /// `weight` includes the wielded weapon; `limit` is what you can carry.
    /// `own`: the server only lets you wield weapons that are in your pack (or already in your hand).
    Stock { gold: f32, items: &'a std::collections::BTreeMap<String, u32>, weight: f32, limit: f32, own: bool },
    /// A vendor's shop as you see it: what it sells (`goods`: units in stock, price per unit now, weight per unit) and what it would pay for things you
    /// carry (`buys`). Prices move with every unit traded, so the total of a purchase is a little more than `ask * qty` when it is large.
    Shop { npc: u16, name: &'a str, kind: &'a str, open: bool, gold: f32, goods: Vec<ShopGood>, buys: Vec<ShopBid> },
    /// You killed a mob and what it dropped went into your pack (as much as fits): gold and goods.
    Drop { gold: f32, items: Vec<(String, u32)> },
    /// The outcome of a `Buy` / `Sell`: how many units changed hands and for how much gold in all; `reason` says why a refused one was ("too far",
    /// "shut", "unknown item", "not in trade", "none left", "too poor", "too heavy", "nothing to sell", "vendor cannot pay").
    Trade { ok: bool, op: &'a str, item: &'a str, units: u32, gold: f32, reason: &'a str },
    /// Your own health (after joining).
    Hp { hp: f32, max: f32 },
}

/// Trim, drop control characters, cap the length (in chars).
pub fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

#[derive(Debug, Serialize)]
pub struct ShopGood {
    pub item: String,
    pub qty: u32,
    pub ask: f32,
    pub w: f32,
}

#[derive(Debug, Serialize)]
pub struct ShopBid {
    pub item: String,
    pub qty: u32,
    pub bid: f32,
}
