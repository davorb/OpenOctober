//! Goods, prices and trade between NPCs (and, later, players).
//!
//! * `Items` is the catalogue of goods from `game/data/items.json` (the game's own base costs, see `tools/build_items.js`).
//! * A `Stock` is gold plus goods: whoever trades has one (an NPC, a vendor, in time a player).
//! * A `Vendor` is a stationary shopkeeper of some `VendorKind`. It deals in the goods groups of its kind, keeps a target stock of some of them and
//!   prices every good by how scarce it is compared with that target (`scarcity`): a vendor with none of something asks more and pays more for it,
//!   one with a glut asks and pays less. Traders and adventurers moving goods between towns are what evens the prices out.
//! * Prices: a vendor sells at 115% and buys at 55% of `cost * scarcity` (the margin is what keeps gold from coming out of thin air when goods go
//!   round and round). Every unit is priced at the stock it finds, so a big sale lowers the price as it goes.
//! * INVENTED, like the combat numbers: the margins, the scarcity curve, vendor gold, restocking and what mobs drop (`loot`). The game ships base
//!   costs but no loot tables.
use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Weapon,
    Armor,
    Food,
    Consumable,
    Herb,
    Material,
    Ammo,
}

impl Group {
    /// What one unit weighs when the game's data has no weight for the item (INVENTED). Gold weighs nothing.
    pub fn weight(self) -> f32 {
        match self {
            Group::Weapon => 6.0,
            Group::Armor => 8.0,
            Group::Food => 0.2,
            Group::Consumable => 0.3,
            Group::Herb => 0.1,
            Group::Material => 1.0,
            Group::Ammo => 0.05,
        }
    }
}

/// What a player can carry, in the game's weight units (a weapon is about 3, a piece of armour 1-5; the wielded weapon counts). INVENTED.
pub const CARRY_LIMIT: f32 = 60.0;
/// Goods the catalogue does not know (a weapon that is not for sale anywhere) weigh this much.
pub const UNKNOWN_WEIGHT: f32 = 5.0;

#[derive(Clone, Debug, Deserialize)]
pub struct Item {
    pub id: String,
    pub cost: f32,
    pub group: Group,
    #[serde(default)]
    pub stack: bool,
    /// The game's "Weight" of one unit (`tools/build_items.js`).
    #[serde(default)]
    pub weight: Option<f32>,
}

#[derive(Default)]
pub struct Items {
    pub list: Vec<Item>,
    index: HashMap<String, usize>,
    by_group: HashMap<Group, Vec<usize>>,
}

impl Items {
    pub fn load(path: &str) -> Result<Items, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        Self::parse(&text).map_err(|e| format!("{path}: {e}"))
    }

    pub fn parse(text: &str) -> Result<Items, serde_json::Error> {
        let list: Vec<Item> = serde_json::from_str(text)?;
        Ok(Self::from_list(list))
    }

    pub fn from_list(list: Vec<Item>) -> Items {
        let mut me = Items { index: HashMap::new(), by_group: HashMap::new(), list };
        for (i, it) in me.list.iter().enumerate() {
            me.index.insert(it.id.clone(), i);
            me.by_group.entry(it.group).or_default().push(i);
        }
        me
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.index.get(id).map(|&i| &self.list[i])
    }

    pub fn group(&self, g: Group) -> impl Iterator<Item = &Item> {
        self.by_group.get(&g).into_iter().flatten().map(|&i| &self.list[i])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VendorKind {
    Merchant,
    Blacksmith,
    Arcanist,
    Market,
}

impl VendorKind {
    pub fn parse(s: &str) -> Option<VendorKind> {
        Some(match s {
            "merchant" => VendorKind::Merchant,
            "blacksmith" => VendorKind::Blacksmith,
            "arcanist" => VendorKind::Arcanist,
            "market" => VendorKind::Market,
            _ => return None,
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            VendorKind::Merchant => "Merchant",
            VendorKind::Blacksmith => "Blacksmith",
            VendorKind::Arcanist => "Arcanist",
            VendorKind::Market => "Marketeer",
        }
    }

    /// The groups of goods this kind of vendor buys and sells.
    pub fn groups(self) -> &'static [Group] {
        match self {
            VendorKind::Merchant => &[Group::Food, Group::Consumable, Group::Ammo, Group::Material],
            VendorKind::Blacksmith => &[Group::Weapon, Group::Armor, Group::Material],
            VendorKind::Arcanist => &[Group::Herb, Group::Consumable],
            VendorKind::Market => &[Group::Food, Group::Herb, Group::Material, Group::Ammo, Group::Consumable],
        }
    }

    pub fn deals_in(self, g: Group) -> bool {
        self.groups().contains(&g)
    }
}

/// Gold and goods.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stock {
    pub gold: f32,
    pub items: BTreeMap<String, u32>,
}

impl Stock {
    pub fn count(&self, item: &str) -> u32 {
        self.items.get(item).copied().unwrap_or(0)
    }

    pub fn add(&mut self, item: &str, qty: u32) {
        if qty > 0 {
            *self.items.entry(item.to_string()).or_insert(0) += qty;
        }
    }

    /// Remove up to `qty`; returns how many were there.
    pub fn take(&mut self, item: &str, qty: u32) -> u32 {
        let Some(have) = self.items.get_mut(item) else { return 0 };
        let n = (*have).min(qty);
        *have -= n;
        if *have == 0 {
            self.items.remove(item);
        }
        n
    }

    /// What the goods are worth at base cost.
    pub fn value(&self, items: &Items) -> f32 {
        self.items.iter().map(|(k, &n)| items.get(k).map_or(0.0, |i| i.cost * n as f32)).sum()
    }

    pub fn units(&self) -> u32 {
        self.items.values().sum()
    }

    /// What the goods weigh (gold does not).
    pub fn weight(&self, items: &Items) -> f32 {
        self.items.iter().map(|(k, &n)| item_weight(items, k) * n as f32).sum()
    }
}

/// What one unit of `item` weighs.
pub fn item_weight(items: &Items, item: &str) -> f32 {
    items.get(item).map_or(UNKNOWN_WEIGHT, |i| i.weight.unwrap_or_else(|| i.group.weight()))
}

/// How much dearer (>1) or cheaper (<1) a good is than its base cost, given the vendor's stock and target of it.
pub fn scarcity(stock: u32, target: u32) -> f32 {
    (((target + 2) as f32) / ((stock + 2) as f32)).powf(0.6).clamp(0.5, 2.5)
}

pub const SELL_MARGIN: f32 = 1.15;
pub const BUY_MARGIN: f32 = 0.55;
/// Vendors pay for goods they have no target for as if they wanted this many.
const UNLISTED_TARGET: u32 = 3;
/// A vendor does not take more than this many units of a good (beyond its target) however cheap.
const GLUT: u32 = 40;

#[derive(Clone, Debug)]
pub struct Vendor {
    pub name: String,
    pub kind: VendorKind,
    /// Where the vendor stands (wire x / z).
    pub post: (f32, f32),
    /// The place (index into `Npcs::places`) it belongs to.
    pub place: usize,
    pub stock: Stock,
    /// What it tries to keep on its shelves, and the gold it works with.
    pub target: BTreeMap<String, u32>,
    pub base_gold: f32,
    /// Seconds since it last restocked.
    pub restock_acc: f32,
    /// The vendor is dead: the shop is shut until it is back (nothing is asked or bid).
    pub closed: bool,
}

/// Why a trade did not (fully) happen.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    UnknownItem,
    /// The vendor does not deal in this good.
    NotInTrade,
    /// Out of stock, or the vendor is out of gold, or the buyer is.
    NothingDone,
}

#[derive(Debug, PartialEq)]
pub struct Deal {
    pub units: u32,
    pub gold: f32,
}

impl Vendor {
    pub fn new(name: String, kind: VendorKind, post: (f32, f32), place: usize, items: &Items, rng: &mut u32) -> Vendor {
        let mut next = || {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 17;
            *rng ^= *rng << 5;
            *rng
        };
        let mut target = BTreeMap::new();
        for &g in kind.groups() {
            let pool: Vec<&Item> = items.group(g).filter(|i| i.cost > 0.0 && i.cost <= 3000.0).collect();
            if pool.is_empty() {
                continue;
            }
            let picks = match g {
                Group::Weapon | Group::Armor => 8,
                Group::Material => 5,
                _ => 6,
            };
            for _ in 0..picks {
                let it = pool[next() as usize % pool.len()];
                // cheap goods are held in numbers, dear ones singly
                let qty = if it.cost < 10.0 { 20 + next() % 30 } else if it.cost < 100.0 { 4 + next() % 8 } else { 1 + next() % 2 };
                target.insert(it.id.clone(), qty);
            }
        }
        let base_gold = 1500.0 + (next() % 3000) as f32;
        let stock = Stock { gold: base_gold, items: target.clone() };
        Vendor { name, kind, post, place, stock, target, base_gold, restock_acc: 0.0, closed: false }
    }

    fn target_of(&self, item: &str) -> u32 {
        self.target.get(item).copied().unwrap_or(UNLISTED_TARGET)
    }

    /// What the vendor asks for one unit of `item` now (`None` if it has none or does not deal in it).
    pub fn ask(&self, items: &Items, item: &str) -> Option<f32> {
        let it = items.get(item)?;
        let have = self.stock.count(item);
        (!self.closed && it.cost > 0.0 && self.kind.deals_in(it.group) && have > 0).then(|| it.cost * scarcity(have, self.target_of(item)) * SELL_MARGIN)
    }

    /// What the vendor pays for one unit of `item` now (`None` if it does not deal in it, has too many already, or cannot afford it).
    pub fn bid(&self, items: &Items, item: &str) -> Option<f32> {
        let it = items.get(item)?;
        let have = self.stock.count(item);
        if self.closed || it.cost <= 0.0 || !self.kind.deals_in(it.group) || have >= self.target_of(item) + GLUT {
            return None;
        }
        let p = it.cost * scarcity(have, self.target_of(item)) * BUY_MARGIN;
        (self.stock.gold >= p).then_some(p)
    }

    /// `seller` sells up to `qty` units of `item` to the vendor, one unit at a time at the price the vendor then pays.
    pub fn buy_from(&mut self, items: &Items, seller: &mut Stock, item: &str, qty: u32) -> Result<Deal, Refusal> {
        let it = items.get(item).ok_or(Refusal::UnknownItem)?;
        if !self.kind.deals_in(it.group) {
            return Err(Refusal::NotInTrade);
        }
        let mut deal = Deal { units: 0, gold: 0.0 };
        for _ in 0..qty.min(seller.count(item)) {
            let Some(p) = self.bid(items, item) else { break };
            seller.take(item, 1);
            self.stock.add(item, 1);
            self.stock.gold -= p;
            seller.gold += p;
            deal.units += 1;
            deal.gold += p;
        }
        if deal.units == 0 {
            Err(Refusal::NothingDone)
        } else {
            Ok(deal)
        }
    }

    /// `buyer` buys up to `qty` units of `item` from the vendor, one unit at a time at the price the vendor then asks, for as long as it has any and
    /// the buyer can pay.
    pub fn sell_to(&mut self, items: &Items, buyer: &mut Stock, item: &str, qty: u32) -> Result<Deal, Refusal> {
        let it = items.get(item).ok_or(Refusal::UnknownItem)?;
        if !self.kind.deals_in(it.group) {
            return Err(Refusal::NotInTrade);
        }
        let mut deal = Deal { units: 0, gold: 0.0 };
        for _ in 0..qty {
            let Some(p) = self.ask(items, item) else { break };
            if buyer.gold < p {
                break;
            }
            self.stock.take(item, 1);
            buyer.add(item, 1);
            buyer.gold -= p;
            self.stock.gold += p;
            deal.units += 1;
            deal.gold += p;
        }
        if deal.units == 0 {
            Err(Refusal::NothingDone)
        } else {
            Ok(deal)
        }
    }

    /// Time passes (about every two minutes): shelves drift back to their targets (empty ones refill, gluts and goods it has no target for sell on
    /// elsewhere), and the vendor's gold to its working level.
    pub fn restock(&mut self) {
        for (item, &t) in &self.target {
            let have = self.stock.count(item);
            if have < t {
                self.stock.add(item, ((t - have) as f32 * 0.25).ceil() as u32);
            } else if have > t {
                self.stock.take(item, ((have - t) as f32 * 0.15).ceil() as u32);
            }
        }
        let unlisted: Vec<String> = self.stock.items.keys().filter(|k| !self.target.contains_key(*k)).cloned().collect();
        for k in unlisted {
            let have = self.stock.count(&k);
            self.stock.take(&k, ((have as f32) * 0.3).ceil() as u32);
        }
        self.stock.gold += (self.base_gold - self.stock.gold) * 0.1;
    }
}

/// What a mob of difficulty `rank` (0..6) drops: gold and up to two goods. INVENTED: the budget grows with the rank, and so does the pool of goods.
pub fn loot(items: &Items, rank: u8, rng: &mut u32) -> (f32, Vec<(String, u32)>) {
    let mut next = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 17;
        *rng ^= *rng << 5;
        (*rng >> 8) as f32 / (1u32 << 24) as f32
    };
    let r = rank as f32 + 1.0;
    let gold = ((1.0 + next() * 3.0) * 2.0 * r.powf(1.6)).round();
    let budget = 15.0 * r * r;
    let mut drops: Vec<(String, u32)> = Vec::new();
    for _ in 0..2 {
        if next() > 0.55 {
            continue;
        }
        let g = match next() {
            x if x < 0.30 => Group::Material,
            x if x < 0.55 => Group::Herb,
            x if x < 0.70 => Group::Food,
            x if x < 0.82 => Group::Consumable,
            x if x < 0.88 => Group::Ammo,
            x if x < 0.95 => Group::Weapon,
            _ => Group::Armor,
        };
        let pool: Vec<&Item> = items.group(g).filter(|i| i.cost > 0.0 && i.cost <= budget).collect();
        if pool.is_empty() {
            continue;
        }
        let it = pool[(next() * pool.len() as f32) as usize % pool.len()];
        let qty = if it.stack { 1 + (next() * 3.0) as u32 } else { 1 };
        drops.push((it.id.clone(), qty));
    }
    (gold, drops)
}

/// A vendor's post as the data file gives it (wire coordinates).
#[derive(Clone, Debug)]
pub struct VendorSpot {
    pub name: String,
    pub kind: VendorKind,
    pub x: f32,
    pub z: f32,
}

#[derive(Deserialize)]
struct VendorFile {
    name: String,
    kind: String,
    x: f32,
    z: f32,
}

/// Read `game/data/vendors.json` (game coordinates, like pois.json: the wire z is the negated z).
pub fn load_vendors(path: &str) -> Result<Vec<VendorSpot>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let rows: Vec<VendorFile> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    Ok(rows.into_iter().filter_map(|r| Some(VendorSpot { name: r.name, kind: VendorKind::parse(&r.kind)?, x: r.x, z: -r.z })).collect())
}

/// Everything that is for sale: the catalogue and the vendors' shelves.
#[derive(Default)]
pub struct Economy {
    pub items: Items,
    pub vendors: Vec<Vendor>,
}
