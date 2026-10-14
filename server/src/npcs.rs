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
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::economy::{self, Economy, Group, Items, Stock, Vendor, VendorSpot};
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

/// A dead body's tombstone and what lies on it (see `world::CORPSE_SECS`). Anybody can take the contents once: NPCs that see it, the owner coming
/// back for its things, players.
#[derive(Clone, Debug)]
pub struct Grave {
    pub cid: u32,
    pub x: f32,
    pub z: f32,
    pub stock: Stock,
}

/// What an NPC does for a living. Adventurers hunt, explore and sell what they find; traders carry goods between towns for profit; vendors stand at
/// their post in a town and are what the others trade with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Adventurer,
    Trader,
    Vendor,
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
    /// Go and look at `to` (a dungeon entrance, an unvisited stretch of wilderness): walk there, then linger for `left` more seconds.
    Explore { to: (f32, f32), left: f32, arrived: bool },
    /// Walk to the vendor `vendor` (index into the economy's vendors), trade when there, then linger `left` more seconds.
    Trade { vendor: usize, left: f32, arrived: bool },
    /// Walk to the tombstone `cid` and take what lies there, then linger `left` more seconds.
    Loot { cid: u32, left: f32, arrived: bool },
}

const WANDER_R: f32 = 45.0;
/// Ground an NPC can use is a connected region of at least this many 2 m cells (a house is a few hundred).
const OPEN_REGION: usize = 1500;
/// Explorers walk at this speed, look at places 400 m to this far away, and remember the areas they have seen in cells of EXPLORE_CELL metres.
const EXPLORE_SPEED: f32 = 2.4;
const EXPLORE_RANGE: (f32, f32) = (400.0, 1600.0);
const EXPLORE_CELL: f32 = 600.0;
/// Beyond this far from the place it is at, an NPC mostly heads back before choosing anything else.
const HOME_RADIUS: f32 = 150.0;
/// Share of the regular population that are traders.
const TRADER_SHARE: f32 = 0.08;
/// An adventurer goes to a vendor when its loot is worth this much, and keeps a few things for itself.
const SELL_WORTH: f32 = 15.0;
const KEEP_FOOD: u32 = 3;
/// A vendor restocks every this many seconds.
const RESTOCK_EVERY: f32 = 120.0;
/// A good has to sell for this much more where the trader is going than it costs here before it is worth carrying.
const TRADE_MARGIN: f32 = 1.2;
const MAX_CARGO: u32 = 80;
/// An adventurer carries at most this many goods: past it the cheapest are left behind (what no vendor on its rounds would take).
const MAX_LOOT: u32 = 40;
/// An NPC notices a tombstone with something on it from this far, and goes for it.
const SIGHT: f32 = 60.0;
/// An NPC goes back for its own things this many times before it gives them up.
const MAX_RETRIEVALS: u8 = 2;
/// An NPC hunts at spawn points this close to the place it is at, and no tougher than `hunt_cap(level)`.
const HUNT_RADIUS: f32 = 2500.0;
const GROUND_CELL: f32 = 2000.0;
/// ASSUMPTION (the game data has no table of who is hostile to whom): the three chaos races are one side, the other three the other, as in Darkfall.
pub fn chaos_race(race: &str) -> bool {
    matches!(race, "ork" | "mir" | "mhm")
}

/// How an adventurer treats players. Fixed from its id, so a saved NPC keeps it: 70% friendly, 20% hostile to the other side, 10% hostile to every player.
/// Vendors and traders are never hostile on sight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Temper {
    Friendly,
    /// Attacks players of the other side.
    Faction,
    /// Attacks every player.
    Outlaw,
}

impl Temper {
    /// The temper an adventurer with this id has.
    pub fn of_adventurer(id: u16) -> Self {
        match id.wrapping_sub(NPC_ID_BASE) % 10 {
            0 => Temper::Outlaw,
            1 | 2 => Temper::Faction,
            _ => Temper::Friendly,
        }
    }
}

/// Vendors can be killed, but it takes an army: they are very hard to hurt, hit very hard and fight back. A dead vendor drops its shelves and purse,
/// its shop stays shut and it is back at its post after VENDOR_DOWN_SECS with empty shelves that fill up again as the vendor restocks.
pub const VENDOR_HP: f32 = 6000.0;
pub const VENDOR_DAMAGE: f32 = 6.0;
pub const VENDOR_DOWN_SECS: f32 = 600.0;

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
const ROUTE_EXPAND: usize = 40000;

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
    /// Weapon type of `info.weapon` ("" = unarmed: such an NPC does not hunt).
    pub group: String,
    /// Seconds left lying dead (0 = alive), until the next blow may be struck.
    pub down: f32,
    pub next_attack: f32,
    /// Mob being fought, the point to walk to for it, how close to get, and seconds without a target (the world sets these each tick).
    pub target: Option<u16>,
    pub chase: Option<(f32, f32)>,
    pub stop: f32,
    pub no_target: f32,
    /// The connected player this NPC is fighting (the world sets it), players it holds a grudge against for hurting it (id, seconds left), and
    /// seconds until it may greet somebody again.
    pub foe: Option<u16>,
    pub grudges: Vec<(u16, f32)>,
    pub greet: f32,
    /// The weapon skill the NPC starts with in whatever type of weapon it is armed with (`equip_all`).
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
    /// Destinations (kind 0 place, 1 hunting ground) it found no route to; it does not pick them again for a while.
    avoid: Vec<(u8, usize)>,
    /// Areas (EXPLORE_CELL squares) it has been to, oldest first.
    explored: Vec<(i32, i32)>,
    pub role: Role,
    /// Gold and goods carried (a vendor's own are on its shelves, in the economy).
    pub stock: Stock,
    /// For a vendor: its index in the economy.
    pub vendor: Option<usize>,
    /// Errands in town: it wants to visit vendors (those it has been to this stay are in `visited`); a trader also remembers where it will go next.
    trade_due: bool,
    visited: Vec<usize>,
    trade_dest: Option<usize>,
    /// Seconds hunting since the last drop, since the last meal / restock.
    loot_acc: f32,
    eat_acc: f32,
    /// Its hit points when it started a loot channel (hurt = interrupted).
    chan_hp: f32,
    /// The tombstone with its own things on it, and how often it has gone back for them.
    grave: Option<u32>,
    retrievals: u8,
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
    /// 0 adventurer, 1 trader, 2 vendor; gold and goods as JSON ({"r_Apple": 3, ...}) - a vendor's are those on its shelves.
    pub role: u8,
    pub gold: f32,
    pub stock: String,
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
    /// NPC time that nobody watches runs this many times faster (1.0 = real time; a debugging aid: visible NPCs always run in real time).
    pub time_scale: f32,
    /// Seconds an NPC needs to take everything from a tombstone (the world copies `Config::loot_secs` here).
    pub loot_secs: f32,
    /// `standable` answers by target cell (places and grounds do not move).
    stand_cache: RefCell<HashMap<(i32, i32), Option<(f32, f32)>>>,
    /// The goods and the vendors; `place_vendors[p]` = the vendors of place `p` (indices into `econ.vendors`).
    econ: RefCell<Economy>,
    place_vendors: Vec<Vec<usize>>,
    /// Weapon id -> (type, rank), for NPCs that shop for a better one.
    weapon_table: HashMap<String, (String, f32)>,
    /// NPCs that changed their weapon in the last step (the world tells whoever sees them).
    pub weapon_changed: RefCell<Vec<u16>>,
    /// Visits to a counter and the gold that changed hands since the stats were last read.
    trades: std::cell::Cell<(u32, f32)>,
    /// Tombstones with something on them (NPCs and players), and the ones emptied since the world last looked.
    graves: RefCell<Vec<Grave>>,
    pub looted: RefCell<Vec<u32>>,
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
    /// 0..1, fixed per NPC: how much it likes to go and look at things.
    pub fn curiosity(&self) -> f32 {
        ((self.id as u32).wrapping_mul(2654435761) >> 16) as f32 % 100.0 / 100.0
    }

    /// Areas (600 m squares) this NPC has explored.
    pub fn explored_cells(&self) -> &[(i32, i32)] {
        &self.explored
    }

    pub fn skill(&self, name: &str) -> f32 {
        self.skills.get(name).copied().unwrap_or(0.0)
    }

    /// The skill of the weapon type it is armed with.
    pub fn melee_skill(&self) -> f32 {
        self.skill(weapon_skill(&self.group))
    }

    /// The toughest mob rank it dares to hunt.
    pub fn hunt_cap(&self) -> u8 {
        hunt_cap(self.melee_skill())
    }

    /// Friendly, hostile to the other side, or hostile to every player. Only adventurers are ever the last two.
    pub fn temper(&self) -> Temper {
        if self.role == Role::Adventurer { Temper::of_adventurer(self.id) } else { Temper::Friendly }
    }

    /// Would attack a player of this side on sight. A grudge is separate, and so is pvp being on.
    pub fn hostile_to(&self, player_chaos: bool) -> bool {
        match self.temper() {
            Temper::Outlaw => true,
            Temper::Faction => chaos_race(&self.info.race) != player_chaos,
            Temper::Friendly => false,
        }
    }
}

impl Npcs {
    pub fn empty() -> Self {
        Npcs::default()
    }

    /// Build the place graph from `pois.json` (towns, dungeons) and `spawns.json` (the racial capitals), then create `count` NPCs.
    /// `seed` makes the population reproducible (a saved world overrides it with the stored state, see `restore`).
    pub fn load(pois: &str, bindstones: &str, count: usize, seed: u32) -> Result<Self, String> {
        let mut me = Self::load_bare(pois, bindstones)?;
        me.populate(count, seed);
        Ok(me)
    }

    fn load_bare(pois: &str, bindstones: &str) -> Result<Self, String> {
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
        Ok(Self::bare(places, &capitals))
    }

    /// Same as `load` from places already in memory (tests): `capitals` are (race index, wire x/z).
    pub fn build(places: Vec<Place>, capitals: &[(usize, (f32, f32))], count: usize, seed: u32) -> Self {
        let mut me = Self::bare(places, capitals);
        me.populate(count, seed);
        me
    }

    /// `build` with the vendors of an economy first (their ids stay the same however many regular NPCs follow).
    pub fn build_with_economy(places: Vec<Place>, capitals: &[(usize, (f32, f32))], items: Items, spots: &[VendorSpot], count: usize, seed: u32) -> Self {
        let mut me = Self::bare(places, capitals);
        me.add_vendors(items, spots, seed ^ 0x9E37_79B9);
        me.populate(count, seed);
        me
    }

    /// `load` with the economy: `items.json` and `vendors.json`.
    pub fn load_with_economy(pois: &str, bindstones: &str, items: &str, vendors: &str, count: usize, seed: u32) -> Result<Self, String> {
        let (items, spots) = (Items::load(items)?, economy::load_vendors(vendors)?);
        let mut me = Self::load_bare(pois, bindstones)?;
        me.add_vendors(items, &spots, seed ^ 0x9E37_79B9);
        me.populate(count, seed);
        Ok(me)
    }

    /// The place graph and nothing else.
    fn bare(places: Vec<Place>, capitals: &[(usize, (f32, f32))]) -> Self {
        let links: Vec<Vec<usize>> = (0..places.len())
            .map(|i| {
                let mut near: Vec<(f32, usize)> = (0..places.len()).filter(|&j| j != i).map(|j| (dist((places[i].x, places[i].z), (places[j].x, places[j].z)), j)).filter(|d| d.0 <= MAX_HOP).collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                near.into_iter().take(LINKS).map(|d| d.1).collect()
            })
            .collect();
        let race_of: Vec<usize> = places.iter().map(|p| capitals.iter().min_by(|a, b| dist((p.x, p.z), a.1).total_cmp(&dist((p.x, p.z), b.1))).map_or(0, |c| c.0)).collect();
        let n_places = places.len();
        Npcs { npcs: Vec::new(), places, links, race_of, tick: 0, grounds: Vec::new(), ground_grid: HashMap::new(), promoted: Vec::new(), revived: Vec::new(), time_scale: 1.0, loot_secs: 3.0, stand_cache: RefCell::new(HashMap::new()), econ: RefCell::new(Economy::default()), place_vendors: vec![Vec::new(); n_places], weapon_table: HashMap::new(), weapon_changed: RefCell::new(Vec::new()), trades: std::cell::Cell::new((0, 0.0)), graves: RefCell::new(Vec::new()), looted: RefCell::new(Vec::new()), nav: None }
    }

    fn blank(&self, idx: usize, name: String, role: Role, home: usize, pos: (f32, f32), rng: &mut u32) -> Npc {
        let (race, gender) = (self.race_of[home], xorshift(rng) as usize % 2);
        let gear = GEAR[xorshift(rng) as usize % GEAR.len()].to_string();
        let id = NPC_ID_BASE + idx as u16;
        let role_name = match role {
            Role::Adventurer => "",
            Role::Trader => "trader",
            Role::Vendor => "vendor",
        };
        let mut npc = Npc {
            id,
            info: PlayerInfo { id, name, race: RACES[race].to_string(), gender: GENDERS[gender].to_string(), gear, weapon: String::new(), drawn: false, npc: true, role: role_name.to_string() },
            pos: PosUpdate { x: pos.0, y: 0.0, z: pos.1, yaw: frac(rng) * 6.28, speed: 0, flags: FLAG_SNAP | FLAG_NPC },
            lod: Lod::Far,
            skills: BTreeMap::from([("Health".to_string(), 10.0 + frac(rng) * 50.0)]),
            hp: 0.0,
            max_hp: 0.0,
            group: String::new(),
            down: 0.0,
            next_attack: 0.0,
            target: None,
            chase: None,
            stop: 2.0,
            no_target: 0.0,
            foe: None,
            grudges: Vec::new(),
            greet: 0.0,
            start_melee: 5.0 + frac(rng) * frac(rng) * 65.0,
            home,
            at: home,
            goal: Goal::Rest { left: frac(rng) * 30.0 },
            fatigue: frac(rng) * 0.5,
            acc: frac(rng) * FAR_EVERY,
            rng: xorshift(rng) | 1,
            path: Vec::new(),
            path_goal: None,
            good: pos,
            avoid: Vec::new(),
            explored: Vec::new(),
            role,
            stock: Stock::default(),
            vendor: None,
            trade_due: false,
            visited: Vec::new(),
            trade_dest: None,
            loot_acc: 0.0,
            eat_acc: 0.0,
            chan_hp: 0.0,
            grave: None,
            retrievals: 0,
        };
        npc.max_hp = if role == Role::Vendor { VENDOR_HP } else { npc_hp(npc.skill("Health")) };
        npc.hp = npc.max_hp;
        npc
    }

    /// Put the vendors at their posts. Must come before `populate`. A vendor belongs to the nearest town within 700 m (others are dropped).
    pub fn add_vendors(&mut self, items: Items, spots: &[VendorSpot], seed: u32) {
        let mut rng = seed | 1;
        let mut vendors = Vec::new();
        for s in spots {
            let near = (0..self.places.len()).filter(|&i| self.places[i].town).map(|i| (dist((self.places[i].x, self.places[i].z), (s.x, s.z)), i)).min_by(|a, b| a.0.total_cmp(&b.0));
            let Some((d, place)) = near else { continue };
            if d > 700.0 || self.npcs.len() >= MAX_NPCS {
                continue;
            }
            let vi = vendors.len();
            let title = format!("{} the {}", s.name, s.kind.title());
            vendors.push(Vendor::new(title.clone(), s.kind, (s.x, s.z), place, &items, &mut rng));
            let mut npc = self.blank(self.npcs.len(), title, Role::Vendor, place, (s.x, s.z), &mut rng);
            npc.vendor = Some(vi);
            npc.goal = Goal::Rest { left: 30.0 };
            self.place_vendors[place].push(vi);
            self.npcs.push(npc);
        }
        *self.econ.borrow_mut() = Economy { items, vendors };
    }

    /// Create `count` regular NPCs (adventurers, and a few traders) living in the towns.
    pub fn populate(&mut self, count: usize, seed: u32) {
        let towns: Vec<usize> = (0..self.places.len()).filter(|&i| self.places[i].town).collect();
        if towns.is_empty() {
            return;
        }
        let has_vendors = !self.econ.borrow().vendors.is_empty();
        let mut rng = seed | 1;
        for _ in 0..count {
            if self.npcs.len() >= MAX_NPCS {
                break;
            }
            let home = towns[xorshift(&mut rng) as usize % towns.len()];
            let name = make_name(&mut rng);
            let p = &self.places[home];
            let pos = (p.x + (frac(&mut rng) - 0.5) * 2.0 * WANDER_R, p.z + (frac(&mut rng) - 0.5) * 2.0 * WANDER_R);
            let draw = frac(&mut rng);
            let role = if has_vendors && draw < TRADER_SHARE { Role::Trader } else { Role::Adventurer };
            let name = if role == Role::Trader { format!("{name} the Trader") } else { name };
            let mut npc = self.blank(self.npcs.len(), name, role, home, pos, &mut rng);
            npc.stock.gold = match role {
                Role::Trader => 600.0 + frac(&mut rng) * 1900.0,
                _ => 20.0 + frac(&mut rng) * 130.0,
            };
            self.think(&mut npc);
            self.npcs.push(npc);
        }
    }

    /// The goods and the vendors' shelves (what a trading UI will read) and, to change them, the same mutably.
    pub fn economy(&self) -> std::cell::Ref<'_, Economy> {
        self.econ.borrow()
    }

    pub fn economy_mut(&self) -> std::cell::RefMut<'_, Economy> {
        self.econ.borrow_mut()
    }

    /// A body fell at (x, z) carrying `stock`: its tombstone. `owner` (an NPC id) will go back for its things.
    pub fn add_grave(&mut self, cid: u32, x: f32, z: f32, stock: Stock, owner: Option<u16>) {
        let has = stock.gold > 0.0 || !stock.items.is_empty();
        self.graves.borrow_mut().push(Grave { cid, x, z, stock });
        if let (Some(id), true) = (owner, has) {
            if let Some(n) = id.checked_sub(NPC_ID_BASE).and_then(|i| self.npcs.get_mut(i as usize)) {
                n.grave = Some(cid);
            }
        }
    }

    /// A tombstone without a body, for tests.
    pub fn add_grave_for_test(&self, cid: u32, stock: Stock) {
        self.graves.borrow_mut().push(Grave { cid, x: 0.0, z: 0.0, stock });
    }

    /// The tombstone rots away.
    pub fn remove_grave(&mut self, cid: u32) {
        self.graves.borrow_mut().retain(|g| g.cid != cid);
    }

    /// Where the tombstone `cid` is and what is on it (`None` once it is empty or gone).
    pub fn grave(&self, cid: u32) -> Option<Grave> {
        self.graves.borrow().iter().find(|g| g.cid == cid && (g.stock.gold > 0.0 || !g.stock.items.is_empty())).cloned()
    }

    /// Take everything from the tombstone `cid` (the world checks that a player is close enough).
    pub fn take_grave(&self, cid: u32) -> Option<Stock> {
        let mut graves = self.graves.borrow_mut();
        let g = graves.iter_mut().find(|g| g.cid == cid && (g.stock.gold > 0.0 || !g.stock.items.is_empty()))?;
        self.looted.borrow_mut().push(cid);
        Some(std::mem::take(&mut g.stock))
    }

    /// Take what fits from the tombstone `cid`: all the gold and as many goods as weigh at most `free` together (the rest stays on the tombstone,
    /// which is gone once it is empty). `None` if nothing could be taken.
    pub fn take_grave_fitting(&self, cid: u32, mut free: f32, items: &Items) -> Option<Stock> {
        let mut graves = self.graves.borrow_mut();
        let g = graves.iter_mut().find(|g| g.cid == cid && (g.stock.gold > 0.0 || !g.stock.items.is_empty()))?;
        let mut taken = Stock { gold: std::mem::take(&mut g.stock.gold), items: Default::default() };
        let keys: Vec<String> = g.stock.items.keys().cloned().collect();
        for k in keys {
            let w = economy::item_weight(items, &k).max(0.0);
            let have = g.stock.count(&k);
            let n = if w <= 0.0 { have } else { have.min((free / w).floor() as u32) };
            if n > 0 {
                g.stock.take(&k, n);
                taken.add(&k, n);
                free -= w * n as f32;
            }
        }
        if g.stock.gold <= 0.0 && g.stock.items.is_empty() {
            self.looted.borrow_mut().push(cid);
        }
        (taken.gold > 0.0 || !taken.items.is_empty()).then_some(taken)
    }

    /// Take up to `qty` of `item` (the gold when `item` is empty) from the tombstone `cid`, as many as weigh at most `free`. `None` if nothing came off.
    pub fn take_grave_item(&self, cid: u32, item: &str, qty: u32, free: f32, items: &Items) -> Option<Stock> {
        let mut graves = self.graves.borrow_mut();
        let g = graves.iter_mut().find(|g| g.cid == cid && (g.stock.gold > 0.0 || !g.stock.items.is_empty()))?;
        let mut taken = Stock::default();
        if item.is_empty() {
            taken.gold = std::mem::take(&mut g.stock.gold);
        } else {
            let w = economy::item_weight(items, item).max(0.0);
            let n = if w <= 0.0 { qty } else { qty.min((free / w).floor() as u32) };
            let n = g.stock.take(item, n);
            taken.add(item, n);
        }
        if g.stock.gold <= 0.0 && g.stock.items.is_empty() {
            self.looted.borrow_mut().push(cid);
        }
        (taken.gold > 0.0 || !taken.items.is_empty()).then_some(taken)
    }

    /// An NPC (not a vendor) is killed: everything it carries, its weapon too, goes onto its tombstone. It gets up at home with a plain weapon of its type.
    pub fn drop_all(&mut self, id: u16) -> Stock {
        let Some(n) = id.checked_sub(NPC_ID_BASE).and_then(|i| self.npcs.get_mut(i as usize)) else { return Stock::default() };
        let mut dropped = std::mem::take(&mut n.stock);
        if let Some(v) = n.vendor {
            // a vendor drops its shelves and its purse, and the shop is shut
            if let Some(ven) = self.econ.borrow_mut().vendors.get_mut(v) {
                dropped = std::mem::take(&mut ven.stock);
                ven.closed = true;
            }
        }
        if !n.info.weapon.is_empty() {
            dropped.add(&std::mem::take(&mut n.info.weapon), 1);
        }
        dropped
    }

    /// The plainest weapon of a type (lowest rank): what an NPC that lost everything picks up.
    fn starter_weapon(&self, group: &str) -> Option<String> {
        self.weapon_table.iter().filter(|(_, w)| w.0 == group).min_by(|a, b| a.1 .1.total_cmp(&b.1 .1).then(a.0.cmp(b.0))).map(|(k, _)| k.clone())
    }

    /// Equip the best weapon of its own type it carries, if it beats the one in hand.
    fn consider_weapon(&self, n: &mut Npc) {
        if n.group.is_empty() {
            return;
        }
        let current = self.weapon_table.get(&n.info.weapon).map_or(-1.0, |w| w.1);
        let best = n.stock.items.keys().filter_map(|k| self.weapon_table.get(k).filter(|w| w.0 == n.group && w.1 > current).map(|w| (w.1, k.clone()))).max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, k)) = best {
            n.stock.take(&k, 1);
            let old = std::mem::replace(&mut n.info.weapon, k);
            if !old.is_empty() {
                n.stock.add(&old, 1);
            }
            self.weapon_changed.borrow_mut().push(n.id);
        }
    }

    /// A tombstone with something on it that `n` can see.
    fn grave_in_sight(&self, n: &Npc) -> Option<u32> {
        self.graves.borrow().iter().filter(|g| (g.stock.gold > 0.0 || !g.stock.items.is_empty()) && !n.avoid.contains(&(2, g.cid as usize))).map(|g| (dist((n.pos.x, n.pos.z), (g.x, g.z)), g.cid)).filter(|d| d.0 <= SIGHT).min_by(|a, b| a.0.total_cmp(&b.0)).map(|d| d.1)
    }

    /// The vendors within `radius` metres of (x, z), nearest first: (index into `economy().vendors`, the id of the vendor's NPC, distance). This is what
    /// a player's trading window will be opened from.
    pub fn vendors_near(&self, x: f32, z: f32, radius: f32) -> Vec<(usize, u16, f32)> {
        let mut v: Vec<(usize, u16, f32)> = self.npcs.iter().filter_map(|n| n.vendor.map(|i| (i, n.id, dist((x, z), (n.pos.x, n.pos.z))))).filter(|t| t.2 <= radius).collect();
        v.sort_by(|a, b| a.2.total_cmp(&b.2));
        v
    }

    /// How many vendors there are (they come first among the NPC ids).
    pub fn vendor_count(&self) -> usize {
        self.econ.borrow().vendors.len()
    }

    pub fn by_id(&self, id: u16) -> Option<&Npc> {
        id.checked_sub(NPC_ID_BASE).and_then(|i| self.npcs.get(i as usize))
    }

    /// Pick the next goal of `n` from its needs (utility scores; all stubs for now): tired -> rest; otherwise mostly travel to a neighbouring
    /// place, sometimes potter around where it is.
    fn think(&self, n: &mut Npc) {
        if n.role == Role::Vendor {
            n.goal = Goal::Rest { left: 30.0 }; // behind the counter
            return;
        }
        let links = &self.links[n.at];
        let rest = n.fatigue * 1.4;
        let wander = if n.role == Role::Trader { 0.1 } else { 0.25 + frac(&mut n.rng) * 0.2 };
        let travel = if links.is_empty() { 0.0 } else { 0.3 + frac(&mut n.rng) * 0.6 };
        let in_town = self.places[n.at].town;
        // far from the place it is at (after exploring or hunting): usually walk back first
        let p = &self.places[n.at];
        if dist((n.pos.x, n.pos.z), (p.x, p.z)) > HOME_RADIUS && frac(&mut n.rng) < 0.85 && !n.avoid.contains(&(0, n.at)) {
            n.goal = Goal::Travel { dest: n.at };
            return;
        }
        // errands in town: the vendors it has not been to yet; a trader then sets off for the town it picked
        if n.trade_due {
            if let Some(v) = self.next_vendor(n) {
                n.goal = Goal::Trade { vendor: v, left: 3.0 + frac(&mut n.rng) * 4.0, arrived: false };
                return;
            }
            n.trade_due = false;
            n.visited.clear();
            if let Some(d) = n.trade_dest.take() {
                n.goal = Goal::Travel { dest: d };
                return;
            }
        }
        // its own things lie on a tombstone somewhere: go back for them (a couple of tries, then they are somebody else's)
        if let Some(cid) = n.grave {
            if self.grave(cid).is_none() || n.retrievals >= MAX_RETRIEVALS {
                n.grave = None;
            } else if n.hp >= n.max_hp * 0.8 && frac(&mut n.rng) < 0.9 {
                n.retrievals += 1;
                n.goal = Goal::Loot { cid, left: 0.0, arrived: false };
                return;
            }
        }
        let explore = if n.fatigue > 0.7 || n.role == Role::Trader { None } else { self.exploration(n).map(|g| (g, 0.15 + 0.75 * n.curiosity() * (1.0 - n.fatigue) * (0.5 + frac(&mut n.rng)))) };
        let hunt = if n.group.is_empty() || n.fatigue > 0.6 || n.hp < n.max_hp * 0.5 { None } else { self.hunting_ground(n).map(|g| (g, 0.35 + frac(&mut n.rng) * 0.55)) };
        n.goal = if let Some((ground, _)) = hunt.filter(|h| h.1 >= wander.max(travel) && h.1 > rest) {
            Goal::Hunt { ground, left: 60.0 + frac(&mut n.rng) * 120.0, fighting: false }
        } else if let Some((to, score)) = explore.filter(|e| e.1 >= wander.max(travel) && e.1 > rest) {
            Goal::Explore { to, left: 10.0 + score * 40.0, arrived: false }
        } else if rest > wander.max(travel) && in_town {
            Goal::Rest { left: 15.0 + n.fatigue * 90.0 * (0.5 + frac(&mut n.rng)) }
        } else if travel >= wander {
            // a dungeon is a place to look at, not to live in: towns are the more likely stop
            let open: Vec<usize> = links.iter().copied().filter(|&p| !n.avoid.contains(&(0, p)) && (n.role != Role::Trader || !self.place_vendors[p].is_empty())).collect();
            let towns: Vec<usize> = open.iter().copied().filter(|&p| self.places[p].town).collect();
            let pool = if !towns.is_empty() && frac(&mut n.rng) < 0.75 { &towns } else { &open };
            if pool.is_empty() {
                Goal::Wander { left: 10.0 + frac(&mut n.rng) * 30.0, to: None } // nowhere it can get to from here
            } else {
                Goal::Travel { dest: pool[xorshift(&mut n.rng) as usize % pool.len()] }
            }
        } else {
            Goal::Wander { left: 10.0 + frac(&mut n.rng) * 30.0, to: None }
        };
    }

    /// The nearest vendor of the place `n` is at that it has not been to this stay (an adventurer calls on three at most).
    fn next_vendor(&self, n: &mut Npc) -> Option<usize> {
        if n.role != Role::Trader && n.visited.len() >= 3 {
            return None;
        }
        let econ = self.econ.borrow();
        let here = (n.pos.x, n.pos.z);
        self.place_vendors[n.at].iter().copied().filter(|v| !n.visited.contains(v) && !econ.vendors[*v].closed).min_by(|&a, &b| dist(here, econ.vendors[a].post).total_cmp(&dist(here, econ.vendors[b].post)))
    }

    /// Arrived at the place `n.at`: does it have business with the vendors here?
    fn on_arrival(&self, n: &mut Npc) {
        n.visited.clear();
        n.trade_due = false;
        if self.place_vendors[n.at].is_empty() {
            return;
        }
        let econ = self.econ.borrow();
        let worth = n.stock.value(&econ.items);
        let food: u32 = n.stock.items.iter().filter(|(k, _)| econ.items.get(k.as_str()).map_or(false, |i| i.group == Group::Food)).map(|(_, &q)| q).sum();
        n.trade_due = match n.role {
            Role::Trader => true,
            Role::Adventurer => worth >= SELL_WORTH || (food < KEEP_FOOD && n.stock.gold > 30.0) || n.stock.gold > 400.0,
            Role::Vendor => false,
        };
    }

    /// Eat the cheapest food carried: a meal takes the edge off the tiredness.
    fn eat(&self, n: &mut Npc) {
        let econ = self.econ.borrow();
        let meal = n.stock.items.keys().filter_map(|k| econ.items.get(k.as_str())).filter(|i| i.group == Group::Food).min_by(|a, b| a.cost.total_cmp(&b.cost)).map(|i| i.id.clone());
        if let Some(m) = meal {
            n.stock.take(&m, 1);
            n.fatigue = (n.fatigue - 0.2).max(0.0);
        }
    }

    /// What a kill of a mob of this rank leaves.
    fn loot_into(&self, n: &mut Npc, rank: u8) {
        let econ = self.econ.borrow();
        let (gold, drops) = economy::loot(&econ.items, rank, &mut n.rng);
        n.stock.gold += gold;
        if n.stock.units() < 2 * MAX_LOOT {
            for (id, q) in drops {
                n.stock.add(&id, q);
            }
        }
    }

    /// An NPC killed a mob of this rank: it takes the loot.
    pub fn loot_kill(&mut self, id: u16, rank: u8) {
        let Some(i) = id.checked_sub(NPC_ID_BASE) else { return };
        let mut npcs = std::mem::take(&mut self.npcs);
        if let Some(n) = npcs.get_mut(i as usize) {
            if n.role == Role::Adventurer {
                self.loot_into(n, rank);
            }
        }
        self.npcs = npcs;
    }

    /// `n` at the counter of vendor `v`: sells what the vendor will take (an adventurer keeps a few meals), then buys: an adventurer food and, from a
    /// blacksmith, a better weapon of its type; a trader whatever sells for much more in the town it is bound for.
    fn do_trade(&self, n: &mut Npc, v: usize) {
        if self.econ.borrow().vendors[v].closed {
            return; // the shop is shut: its vendor is dead
        }
        let gold_before = n.stock.gold;
        self.do_trade_inner(n, v);
        let (count, gold) = self.trades.get();
        self.trades.set((count + 1, gold + (n.stock.gold - gold_before).abs()));
    }

    fn do_trade_inner(&self, n: &mut Npc, v: usize) {
        let mut econ = self.econ.borrow_mut();
        let Economy { items, vendors } = &mut *econ;
        // a trader first picks where it is going: a neighbouring town with vendors
        if n.role == Role::Trader && n.trade_dest.is_none() {
            let opts: Vec<usize> = self.links[n.at].iter().copied().filter(|&p| self.places[p].town && !self.place_vendors[p].is_empty() && !n.avoid.contains(&(0, p))).collect();
            if !opts.is_empty() {
                n.trade_dest = Some(opts[xorshift(&mut n.rng) as usize % opts.len()]);
            }
        }
        // what the town it is bound for would pay for each good (computed before anything is moved)
        let mut cands: Vec<(String, f32)> = Vec::new();
        if let (Role::Trader, Some(d)) = (n.role, n.trade_dest) {
            for item in vendors[v].stock.items.keys() {
                let best_bid = self.place_vendors[d].iter().filter_map(|&dv| vendors[dv].bid(items, item)).fold(0.0, f32::max);
                if best_bid > 0.0 {
                    cands.push((item.clone(), best_bid));
                }
            }
        }
        let ven = &mut vendors[v];
        // sell
        let mut kept = 0u32;
        let list: Vec<(String, u32)> = n.stock.items.iter().map(|(k, &q)| (k.clone(), q)).collect();
        for (item, qty) in list {
            let Some(it) = items.get(&item) else { continue };
            let mut q = qty;
            if n.role == Role::Adventurer && it.group == Group::Food {
                let keep = KEEP_FOOD.saturating_sub(kept).min(q);
                kept += keep;
                q -= keep;
            }
            if q > 0 {
                let _ = ven.buy_from(items, &mut n.stock, &item, q);
            }
        }
        match n.role {
            Role::Adventurer => {
                if ven.kind.deals_in(Group::Food) {
                    for _ in 0..KEEP_FOOD {
                        let have: u32 = n.stock.items.iter().filter(|(k, _)| items.get(k.as_str()).map_or(false, |i| i.group == Group::Food)).map(|(_, &q)| q).sum();
                        if have >= KEEP_FOOD || n.stock.gold < 20.0 {
                            break;
                        }
                        let cheapest = ven.stock.items.keys().filter(|k| items.get(k.as_str()).map_or(false, |i| i.group == Group::Food)).filter_map(|k| ven.ask(items, k).map(|a| (a, k.clone()))).min_by(|a, b| a.0.total_cmp(&b.0));
                        match cheapest {
                            Some((_, k)) => {
                                let _ = ven.sell_to(items, &mut n.stock, &k, 1);
                            }
                            None => break,
                        }
                    }
                }
                if ven.kind.deals_in(Group::Weapon) {
                    self.try_upgrade(n, ven, items);
                }
                while n.stock.units() > MAX_LOOT {
                    let junk = n.stock.items.keys().min_by(|a, b| items.get(a.as_str()).map_or(0.0, |i| i.cost).total_cmp(&items.get(b.as_str()).map_or(0.0, |i| i.cost))).cloned();
                    match junk {
                        Some(k) => {
                            let q = n.stock.count(&k);
                            n.stock.take(&k, q);
                        }
                        None => break,
                    }
                }
            }
            Role::Trader => {
                let mut cands = cands;
                cands.sort_by(|a, b| {
                    let m = |c: &(String, f32)| ven.ask(items, &c.0).map_or(0.0, |ask| c.1 / ask);
                    m(b).total_cmp(&m(a))
                });
                for (item, best_bid) in cands {
                    while n.stock.units() < MAX_CARGO {
                        let Some(ask) = ven.ask(items, &item) else { break };
                        if best_bid / ask < TRADE_MARGIN || n.stock.gold < ask {
                            break;
                        }
                        if ven.sell_to(items, &mut n.stock, &item, 1).is_err() {
                            break;
                        }
                    }
                }
            }
            Role::Vendor => {}
        }
    }

    /// A better weapon of its own type from the blacksmith, if it can use and afford one.
    fn try_upgrade(&self, n: &mut Npc, ven: &mut Vendor, items: &Items) {
        let current = self.weapon_table.get(&n.info.weapon).map_or(0.0, |w| w.1);
        let allowed = 15.0 + 0.8 * n.melee_skill();
        let best = ven
            .stock
            .items
            .keys()
            .filter_map(|k| self.weapon_table.get(k).filter(|w| w.0 == n.group && w.1 > current && w.1 <= allowed).map(|w| (w.1, k.clone())))
            .filter(|(_, k)| ven.ask(items, k).map_or(false, |a| a <= n.stock.gold * 0.85))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, k)) = best {
            if ven.sell_to(items, &mut n.stock, &k, 1).is_ok() {
                n.stock.take(&k, 1);
                n.info.weapon = k;
                self.weapon_changed.borrow_mut().push(n.id);
            }
        }
    }

    /// Somewhere to go and look at, from the place `n` is at: a dungeon within reach, or a spot of wilderness in an area it has not seen yet.
    fn exploration(&self, n: &mut Npc) -> Option<(f32, f32)> {
        let p = &self.places[n.at];
        let seen = |x: f32, z: f32, n: &Npc| n.explored.contains(&((x / EXPLORE_CELL).floor() as i32, (z / EXPLORE_CELL).floor() as i32));
        // dungeons first (a third of the time): the entrance of one it has not looked at yet
        if frac(&mut n.rng) < 0.33 {
            let places: Vec<&Place> = self.places.iter().filter(|q| !q.town && dist((p.x, p.z), (q.x, q.z)) < 3000.0 && !seen(q.x, q.z, n)).collect();
            if !places.is_empty() {
                let q = places[xorshift(&mut n.rng) as usize % places.len()];
                return Some((q.x, q.z));
            }
        }
        let mut fallback = None;
        for _ in 0..6 {
            let a = frac(&mut n.rng) * std::f32::consts::TAU;
            let r = EXPLORE_RANGE.0 + frac(&mut n.rng) * (EXPLORE_RANGE.1 - EXPLORE_RANGE.0);
            let t = (p.x + a.cos() * r, p.z + a.sin() * r);
            if !seen(t.0, t.1, n) {
                return Some(t);
            }
            fallback = Some(t);
        }
        fallback
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
                    if gr.2 <= cap && dist((p.x, p.z), (gr.0, gr.1)) <= HUNT_RADIUS && !n.avoid.contains(&(1, g)) {
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
        let mut ids: Vec<(&String, &crate::combat::WeaponDef)> = weapons.defs.iter().filter(|(_, d)| d.obtainable() && d.group.as_deref().map_or(false, |g| is_melee(g) && g != "shield")).collect();
        ids.sort_by(|a, b| a.0.cmp(b.0));
        if ids.is_empty() {
            return;
        }
        self.weapon_table = weapons.defs.iter().filter_map(|(k, d)| Some((k.clone(), (d.group.clone()?, d.rank?)))).collect();
        for n in self.npcs.iter_mut().filter(|n| n.role == Role::Adventurer) {
            let max_rank = 10.0 + n.start_melee * 0.7;
            let fit: Vec<_> = ids.iter().filter(|(_, d)| d.rank.unwrap_or(99.0) <= max_rank).collect();
            let pool: Vec<_> = if fit.is_empty() { ids.iter().collect() } else { fit };
            let (id, def) = pool[xorshift(&mut n.rng) as usize % pool.len()];
            n.info.weapon = (*id).clone();
            n.group = def.group.clone().unwrap_or_default();
            let start = n.start_melee;
            n.skills.entry(weapon_skill(&n.group).to_string()).or_insert(start);
        }
        // the vendors are armed with the best melee weapon there is, and are masters of it
        let best = ids.iter().max_by(|a, b| a.1.rank.unwrap_or(0.0).total_cmp(&b.1.rank.unwrap_or(0.0)).then(b.0.cmp(a.0))).map(|(id, d)| ((*id).clone(), d.group.clone().unwrap_or_default()));
        if let Some((id, group)) = best {
            for n in self.npcs.iter_mut().filter(|n| n.role == Role::Vendor) {
                n.info.weapon = id.clone();
                n.group = group.clone();
                n.skills.insert(weapon_skill(&group).to_string(), 100.0);
            }
        }
    }

    /// Raise a skill of `n` by `amount` (capped at 100); Health also raises the hit points.
    fn train(&self, n: &mut Npc, skill: &str, amount: f32) {
        if n.role == Role::Vendor {
            return; // a vendor is as good as it gets
        }
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
        self.by_id(id).map_or(1.0, |n| skill_damage(n.melee_skill()) * if n.role == Role::Vendor { VENDOR_DAMAGE } else { 1.0 })
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
        n.foe = None;
        n.grudges.clear();
        n.target = None;
        n.chase = None;
        n.path.clear();
        n.path_goal = None;
        if n.role == Role::Adventurer && n.info.weapon.is_empty() && !n.group.is_empty() {
            if let Some(w) = self.starter_weapon(&n.group) {
                n.info.weapon = w;
                self.weapon_changed.borrow_mut().push(n.id);
            }
        }
        n.at = n.home;
        let mut home = (self.places[n.home].x, self.places[n.home].z);
        if let Some(v) = n.vendor {
            // back at its post, the shop open again with empty shelves and a little gold
            if let Some(ven) = self.econ.borrow_mut().vendors.get_mut(v) {
                ven.closed = false;
                ven.stock.gold = ven.base_gold * 0.2;
                home = ven.post;
            }
        }
        n.pos.x = home.0;
        n.pos.z = home.1;
        n.pos.speed = 0;
        n.good = home;
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

    /// Where an active NPC actually goes to for `target`: the nearest cell of open ground (a town's centre may be inside a building).
    fn standable(&self, target: (f32, f32), active: bool) -> (f32, f32) {
        let (Some(nav), true) = (&self.nav, active) else { return target };
        let key = ((target.0 / 2.0).floor() as i32, (target.1 / 2.0).floor() as i32);
        let cached = self.stand_cache.borrow().get(&key).copied();
        let found = cached.unwrap_or_else(|| {
            let p = nav.snap_open(target, 40, OPEN_REGION);
            self.stand_cache.borrow_mut().insert(key, p);
            p
        });
        found.unwrap_or(target)
    }

    /// The goal can't be reached (no route): wait a moment, then think again.
    fn give_up(&self, n: &mut Npc) {
        let failed = match n.goal {
            Goal::Travel { dest } => Some((0u8, dest)),
            Goal::Hunt { ground, .. } => Some((1u8, ground)),
            _ => None,
        };
        if let Some(f) = failed {
            if !n.avoid.contains(&f) {
                n.avoid.push(f);
                if n.avoid.len() > 6 {
                    n.avoid.remove(0);
                }
            }
        }
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
        // a tombstone in sight with something on it: anybody not busy fighting goes for it
        if n.role != Role::Vendor && n.foe.is_none() && !matches!(n.goal, Goal::Loot { .. } | Goal::Hunt { fighting: true, .. }) {
            if let Some(cid) = self.grave_in_sight(n) {
                n.goal = Goal::Loot { cid, left: 0.0, arrived: false };
                n.path.clear();
                n.path_goal = None;
            }
        }
        // a vendor that has been drawn away (guarding its stall, or in a fight) walks back to its post
        if active && n.role == Role::Vendor && n.foe.is_none() {
            let post = n.vendor.and_then(|v| self.econ.borrow().vendors.get(v).map(|x| x.post)).unwrap_or((n.pos.x, n.pos.z));
            match n.chase {
                Some(c) if dist((n.pos.x, n.pos.z), c) > n.stop => {
                    let _ = self.walk(n, c, TRAVEL_SPEED, dt, false);
                    return;
                }
                Some(c) => {
                    n.pos.speed = 0;
                    n.pos.yaw = (n.pos.x - c.0).atan2(n.pos.z - c.1);
                    return;
                }
                None if dist((n.pos.x, n.pos.z), post) > 1.0 => {
                    let _ = self.walk(n, post, TRAVEL_SPEED, dt, false);
                    return;
                }
                None => {}
            }
        }
        if active && n.foe.is_some() {
            // in a fight with a player: whatever it was doing waits while it walks up to him (the world sets `chase`)
            match n.chase {
                Some(c) if dist((n.pos.x, n.pos.z), c) > n.stop => {
                    let _ = self.walk(n, c, TRAVEL_SPEED, dt, false);
                }
                Some(c) => {
                    n.pos.speed = 0;
                    n.pos.yaw = (n.pos.x - c.0).atan2(n.pos.z - c.1);
                }
                None => n.pos.speed = 0,
            }
            return;
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
                            // and a drop now and then
                            n.loot_acc += used;
                            while n.loot_acc >= 25.0 {
                                n.loot_acc -= 25.0;
                                self.loot_into(n, rank);
                            }
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
                Goal::Explore { to, left, arrived } => {
                    if !arrived {
                        let target = self.standable(to, active);
                        let before = (n.pos.x, n.pos.z);
                        let Some((used, reached)) = self.walk(n, target, EXPLORE_SPEED, dt, active) else {
                            self.give_up(n);
                            continue;
                        };
                        n.fatigue = (n.fatigue + dist(before, (n.pos.x, n.pos.z)) / 12000.0).min(1.0);
                        dt -= used;
                        if reached {
                            n.goal = Goal::Explore { to, left, arrived: true };
                        }
                    } else {
                        // looking around: standing still, turning slowly
                        let used = dt.min(left);
                        n.pos.speed = 0;
                        n.pos.yaw += used * 0.6;
                        dt -= used;
                        if left - used <= 1e-3 {
                            let cell = ((n.pos.x / EXPLORE_CELL).floor() as i32, (n.pos.z / EXPLORE_CELL).floor() as i32);
                            if !n.explored.contains(&cell) {
                                n.explored.push(cell);
                                if n.explored.len() > 48 {
                                    n.explored.remove(0);
                                }
                            }
                            self.think(n);
                        } else {
                            n.goal = Goal::Explore { to, left: left - used, arrived: true };
                        }
                    }
                }
                Goal::Loot { cid, left, arrived } => {
                    if !arrived {
                        let Some(g) = self.grave(cid) else {
                            self.think(n); // somebody else got there first, or it rotted
                            continue;
                        };
                        let target = self.standable((g.x, g.z), active);
                        let Some((used, reached)) = self.walk(n, target, TRAVEL_SPEED, dt, active) else {
                            n.avoid.push((2, cid as usize)); // cannot get to it
                            if n.avoid.len() > 6 {
                                n.avoid.remove(0);
                            }
                            self.think(n);
                            continue;
                        };
                        dt -= used;
                        if reached {
                            // at the body: the channel starts (hurt during it, and it stops)
                            n.chan_hp = n.hp;
                            n.goal = Goal::Loot { cid, left: self.loot_secs, arrived: true };
                        }
                    } else {
                        // taking everything from the tombstone: standing still until the channel is over
                        let used = dt.min(left);
                        n.pos.speed = 0;
                        dt -= used;
                        if n.hp < n.chan_hp - 0.01 {
                            self.think(n); // hurt: interrupted
                        } else if left - used <= 1e-3 {
                            if let Some(stock) = self.take_grave(cid) {
                                n.stock.gold += stock.gold;
                                for (k, q) in stock.items {
                                    n.stock.add(&k, q);
                                }
                                if n.role == Role::Adventurer {
                                    self.consider_weapon(n);
                                }
                                n.retrievals = 0;
                                if n.grave == Some(cid) {
                                    n.grave = None;
                                }
                            }
                            self.think(n);
                        } else {
                            n.goal = Goal::Loot { cid, left: left - used, arrived: true };
                        }
                    }
                }
                Goal::Trade { vendor, left, arrived } => {
                    let post = self.econ.borrow().vendors[vendor].post;
                    if !arrived {
                        let target = self.standable(post, active);
                        let Some((used, reached)) = self.walk(n, target, EXPLORE_SPEED, dt, active) else {
                            n.visited.push(vendor); // cannot get to the counter: another vendor
                            self.think(n);
                            continue;
                        };
                        dt -= used;
                        if reached {
                            self.do_trade(n, vendor);
                            n.visited.push(vendor);
                            n.goal = Goal::Trade { vendor, left, arrived: true };
                        }
                    } else {
                        // haggling: facing the vendor
                        let used = dt.min(left);
                        n.pos.speed = 0;
                        n.pos.yaw = (n.pos.x - post.0).atan2(n.pos.z - post.1);
                        dt -= used;
                        if left - used <= 1e-3 {
                            self.think(n);
                        } else {
                            n.goal = Goal::Trade { vendor, left: left - used, arrived: true };
                        }
                    }
                }
                Goal::Rest { left } => {
                    let used = left.min(dt);
                    // a vendor restocks its shelves now and then; anybody else eats a meal when tired, if it has one
                    n.eat_acc += used;
                    if let Some(v) = n.vendor {
                        if n.eat_acc >= RESTOCK_EVERY {
                            n.eat_acc = 0.0;
                            self.econ.borrow_mut().vendors[v].restock();
                        }
                    } else if n.eat_acc >= 45.0 {
                        n.eat_acc = 0.0;
                        if n.fatigue > 0.2 {
                            self.eat(n);
                        }
                    }
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
                        self.on_arrival(n);
                        self.think(n);
                    }
                }
            }
        }
    }

    /// An NPC that becomes visible stands on walkable ground: if the abstract walk left it in water or inside a wall, move it to the nearest free cell.
    /// Returns false (and leaves it alone) when there is no free cell within 80 m.
    fn land(&self, n: &mut Npc) -> bool {
        if n.role == Role::Vendor {
            return true; // stands at its post, which may well be inside a building
        }
        n.path.clear();
        n.path_goal = None;
        let Some(nav) = &self.nav else { return true };
        match nav.snap_open((n.pos.x, n.pos.z), 40, OPEN_REGION) {
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
        if let (Some(nav), true) = (&self.nav, n.role != Role::Vendor) {
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
                n.acc += dt * self.time_scale;
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
            n.acc += if lod == Lod::Active && n.lod == Lod::Active { dt } else { dt * self.time_scale };
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

    /// (visits to a counter, gold that changed hands) since the last call, and (gold on the shelves, gold carried, goods carried) now.
    pub fn trade_stats(&self) -> ((u32, f32), (f32, f32, u32)) {
        let t = self.trades.replace((0, 0.0));
        let econ = self.econ.borrow();
        let shelves = econ.vendors.iter().map(|v| v.stock.gold).sum();
        let carried = self.npcs.iter().filter(|n| n.vendor.is_none()).map(|n| n.stock.gold).sum();
        let cargo = self.npcs.iter().filter(|n| n.vendor.is_none()).map(|n| n.stock.units()).sum();
        (t, (shelves, carried, cargo))
    }

    /// How many NPCs are resting / wandering / travelling / walking to a hunt / fighting / exploring (for the stats line).
    pub fn goal_counts(&self) -> [u32; 8] {
        let mut c = [0u32; 8];
        for n in &self.npcs {
            c[match n.goal {
                Goal::Rest { .. } => 0,
                Goal::Wander { .. } => 1,
                Goal::Travel { .. } => 2,
                Goal::Hunt { fighting: false, .. } => 3,
                Goal::Hunt { fighting: true, .. } => 4,
                Goal::Explore { .. } => 5,
                Goal::Trade { .. } => 6,
                Goal::Loot { .. } => 7,
            }] += 1;
        }
        c
    }

    /// NPCs the players can currently see (sent in the player snapshots).
    pub fn active(&self) -> impl Iterator<Item = &Npc> {
        self.npcs.iter().filter(|n| n.lod == Lod::Active)
    }

    /// Everything about every NPC that has to survive a restart.
    pub fn states(&self) -> Vec<NpcState> {
        let econ = self.econ.borrow();
        self.npcs
            .iter()
            .map(|n| {
                // a vendor's gold and goods are the ones on its shelves
                let own = n.vendor.and_then(|v| econ.vendors.get(v)).map_or(&n.stock, |v| &v.stock);
                let (role, gold, stock) = (n.role as u8, own.gold, serde_json::to_string(&own.items).unwrap_or_default());
                let (kind, a, dest) = match n.goal {
                    Goal::Rest { left } => (0, left, -1),
                    Goal::Wander { left, .. } => (1, left, -1),
                    Goal::Travel { dest } => (2, 0.0, dest as i64),
                    Goal::Hunt { ground, left, .. } => (3, left, ground as i64),
                    Goal::Explore { .. } | Goal::Trade { .. } | Goal::Loot { .. } => (0, 1.0, -1), // an exploration or errand in progress is not saved: it rests a moment and chooses again
                };
                NpcState { idx: (n.id - NPC_ID_BASE) as u32, name: n.info.name.clone(), race: n.info.race.clone(), gender: n.info.gender.clone(), gear: n.info.gear.clone(), skills: serde_json::to_string(&n.skills).unwrap_or_default(), weapon: n.info.weapon.clone(), role, gold, stock, home: n.home, at: n.at, goal_kind: kind, goal_a: a, goal_dest: dest, fatigue: n.fatigue, x: n.pos.x, z: n.pos.z, yaw: n.pos.yaw, rng: n.rng }
            })
            .collect()
    }

    /// Put saved NPCs back (rows that don't fit the current population or place list are skipped). Returns how many were restored.
    pub fn restore(&mut self, states: &[NpcState]) -> usize {
        let places = self.places.len();
        let mut restored = 0;
        for st in states {
            let Some(n) = self.npcs.get_mut(st.idx as usize) else { continue };
            if st.role != n.role as u8 {
                continue; // the layout of vendors and traders changed: this row belongs to somebody else
            }
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
            n.max_hp = if n.role == Role::Vendor { VENDOR_HP } else { npc_hp(n.skill("Health")) };
            n.hp = n.max_hp;
            if !st.weapon.is_empty() {
                n.info.weapon = st.weapon.clone();
            }
            let goods: BTreeMap<String, u32> = serde_json::from_str(&st.stock).unwrap_or_default();
            match n.vendor {
                Some(v) => {
                    if let Some(ven) = self.econ.borrow_mut().vendors.get_mut(v) {
                        ven.stock = Stock { gold: st.gold, items: goods };
                    }
                }
                None => n.stock = Stock { gold: st.gold, items: goods },
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
