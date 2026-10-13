//! NPCs that trade: vendors at their posts, adventurers selling loot and buying meals and better weapons, traders moving goods between towns.
use game_server::combat::Weapons;
use game_server::economy::*;
use game_server::nav::{Nav, NavTile};
use game_server::npcs::*;

const ITEMS: &str = r#"[
 {"id":"r_Apple","cost":2,"group":"food","stack":true},{"id":"r_Bread","cost":4,"group":"food","stack":true},
 {"id":"r_Hopta Root","cost":25,"group":"herb","stack":true},{"id":"r_Iron","cost":10,"group":"material","stack":true},
 {"id":"r_Elixir","cost":40,"group":"consumable","stack":true},
 {"id":"r_Iron Sword","cost":80,"group":"weapon"},{"id":"r_Steel Sword","cost":220,"group":"weapon"},{"id":"r_War Sword","cost":900,"group":"weapon"}]"#;
const WEAPONS: &str = r#"{"weapons":{"r_Iron Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":20},
 "r_Steel Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":35},"r_War Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":70}},"clipDur":{}}"#;

fn place(name: &str, x: f32, z: f32) -> Place {
    Place { name: name.into(), x, z, town: true }
}

fn spot(name: &str, kind: VendorKind, x: f32, z: f32) -> VendorSpot {
    VendorSpot { name: name.into(), kind, x, z }
}

/// Two towns 2 km apart, each with a merchant and the western one with a blacksmith too.
fn world(count: usize, seed: u32) -> Npcs {
    let places = vec![place("A", 0.0, 0.0), place("B", 2000.0, 0.0)];
    let spots = vec![spot("Ann", VendorKind::Merchant, 20.0, 5.0), spot("Smith", VendorKind::Blacksmith, -25.0, 10.0), spot("Bo", VendorKind::Merchant, 2015.0, 0.0)];
    Npcs::build_with_economy(places, &[(0, (0.0, 0.0))], Items::parse(ITEMS).unwrap(), &spots, count, seed)
}

fn far(w: &mut Npcs, seconds: f32) {
    for _ in 0..(seconds / 5.0) as usize {
        w.step(5.0, &[(-50_000.0, 0.0)], 900.0);
    }
}

#[test]
fn vendors_stand_at_their_posts_and_restock() {
    let mut w = world(10, 3);
    assert_eq!(w.vendor_count(), 3);
    let vendors: Vec<_> = w.npcs.iter().filter(|n| n.role == Role::Vendor).collect();
    assert_eq!(vendors.len(), 3, "vendors come first");
    assert!(w.npcs.iter().take(3).all(|n| n.role == Role::Vendor) && w.npcs.iter().skip(3).all(|n| n.role != Role::Vendor));
    assert!(vendors[0].info.name.contains("the Merchant") && vendors[0].info.role == "vendor");
    let posts: Vec<(f32, f32)> = vendors.iter().map(|n| (n.pos.x, n.pos.z)).collect();
    // empty a shelf and let time pass: it refills, and nobody leaves his post
    let item = w.economy().vendors[0].target.keys().next().cloned().unwrap();
    w.economy_mut().vendors[0].stock.items.remove(&item);
    far(&mut w, 1800.0);
    assert!(w.economy().vendors[0].stock.count(&item) > 0, "restocked");
    for (n, p) in w.npcs.iter().take(3).zip(posts) {
        assert_eq!((n.pos.x, n.pos.z), p, "{} stays at his post", n.info.name);
    }
}

#[test]
fn a_player_can_find_and_trade_with_the_vendors_near_him() {
    let w = world(2, 7);
    let near = w.vendors_near(0.0, 0.0, 60.0);
    assert_eq!(near.len(), 2, "the merchant and the blacksmith of town A, not the one in B: {near:?}");
    assert!(near[0].2 <= near[1].2, "nearest first");
    assert!(w.by_id(near[0].1).unwrap().vendor == Some(near[0].0));
    // a player has no inventory yet; a plain Stock does the job of one: buy an apple from the merchant and sell it back
    let merchant = near.iter().find(|n| w.economy().vendors[n.0].kind == VendorKind::Merchant).unwrap().0;
    let mut me = Stock { gold: 50.0, items: Default::default() };
    let mut eco = w.economy_mut();
    let Economy { items, vendors } = &mut *eco;
    let apple = vendors[merchant].stock.items.keys().find(|k| items.get(k).unwrap().group == Group::Food).cloned().expect("a merchant sells food");
    let bought = vendors[merchant].sell_to(items, &mut me, &apple, 2).unwrap();
    assert_eq!(bought.units, 2);
    assert!(me.gold < 50.0 && me.count(&apple) == 2);
    let sold = vendors[merchant].buy_from(items, &mut me, &apple, 2).unwrap();
    assert!(sold.gold < bought.gold, "the vendor keeps his margin");
}

#[test]
fn adventurers_sell_their_loot_and_buy_meals() {
    let mut w = world(1, 11);
    let i = w.npcs.iter().position(|n| n.role != Role::Vendor).unwrap();
    {
        let n = &mut w.npcs[i];
        n.role = Role::Adventurer;
        n.stock = Stock { gold: 50.0, items: Default::default() };
        n.stock.add("r_Iron", 6);
        n.stock.add("r_Hopta Root", 2);
        // far from the town it is about to reach
        n.pos.x = 1000.0;
        n.pos.z = 700.0;
        n.at = 1;
        n.goal = Goal::Travel { dest: 0 };
    }
    let (gold0, iron_vendor0) = (w.npcs[i].stock.gold, w.economy().vendors[0].stock.count("r_Iron"));
    far(&mut w, 1800.0);
    let n = &w.npcs[i];
    assert!(n.stock.count("r_Iron") == 0, "the iron was sold: {:?}", n.stock.items);
    let eco = w.economy();
    let _ = iron_vendor0;
    let food: u32 = n.stock.items.iter().filter(|(k, _)| eco.items.get(k).map_or(false, |it| it.group == Group::Food)).map(|(_, &q)| q).sum();
    assert!(n.stock.gold > gold0 - 1.0 || food > 0, "sold for gold or bought meals: gold {} -> {}", gold0, n.stock.gold);
}

#[test]
fn a_blacksmith_sells_a_better_weapon_to_a_skilled_adventurer() {
    let mut w = world(1, 21);
    let weapons = Weapons::parse(WEAPONS).unwrap();
    w.equip_all(&weapons);
    let i = w.npcs.iter().position(|n| n.role != Role::Vendor).unwrap();
    {
        let n = &mut w.npcs[i];
        n.role = Role::Adventurer;
        n.info.weapon = "r_Iron Sword".into();
        n.group = "sword".into();
        n.skills.insert("Swords".into(), 60.0); // may use a rank up to 15 + 0.8 * 60 = 63
        n.stock = Stock { gold: 2000.0, items: Default::default() };
        n.pos.x = 1000.0;
        n.pos.z = 700.0;
        n.at = 1;
        n.goal = Goal::Travel { dest: 0 };
    }
    // the smith holds the steel and war swords
    {
        let mut eco = w.economy_mut();
        let smith = eco.vendors.iter_mut().find(|v| v.kind == VendorKind::Blacksmith).unwrap();
        for s in ["r_Steel Sword", "r_War Sword"] {
            smith.target.insert(s.into(), 1);
            smith.stock.add(s, 1);
        }
    }
    far(&mut w, 1800.0);
    let n = &w.npcs[i];
    assert_eq!(n.info.weapon, "r_Steel Sword", "bought the best one it can use (rank 35; the war sword is rank 70)");
    assert!(w.weapon_changed.borrow().contains(&n.id));
    assert!(n.stock.gold < 2000.0);
}

#[test]
fn traders_carry_goods_to_where_they_are_dear_and_prices_even_out() {
    let mut w = world(8, 5);
    // west: a glut of herbs; east: none at all
    {
        let mut eco = w.economy_mut();
        let (a, b) = (0usize, 2usize);
        for v in [a, b] {
            eco.vendors[v].target.insert("r_Hopta Root".into(), 30);
        }
        // the merchants of this fixture do not deal in herbs: make them arcanists
        eco.vendors[a].kind = VendorKind::Market;
        eco.vendors[b].kind = VendorKind::Market;
        eco.vendors[a].stock.items.insert("r_Hopta Root".into(), 60);
        eco.vendors[b].stock.items.remove("r_Hopta Root");
        eco.vendors[a].stock.gold = 9000.0;
        eco.vendors[b].stock.gold = 9000.0;
    }
    let price_gap = |w: &Npcs| {
        let eco = w.economy();
        let ask_b = eco.vendors[2].ask(&eco.items, "r_Hopta Root");
        let bid_a = eco.vendors[0].bid(&eco.items, "r_Hopta Root");
        (ask_b, bid_a, eco.vendors[2].stock.count("r_Hopta Root"), eco.vendors[0].stock.count("r_Hopta Root"))
    };
    let before = price_gap(&w);
    assert!(before.0.is_none(), "east has none to sell");
    // three traders start in the west, each with 3000 gold, about to arrive in the town
    let mut made = 0;
    for n in w.npcs.iter_mut().filter(|n| n.role != Role::Vendor).take(3) {
        n.role = Role::Trader;
        n.info.role = "trader".into();
        n.stock = Stock { gold: 3000.0, items: Default::default() };
        n.pos.x = 300.0;
        n.pos.z = 0.0;
        n.at = 0;
        n.goal = Goal::Travel { dest: 0 };
        made += 1;
    }
    assert_eq!(made, 3);
    let gold0: f32 = w.npcs.iter().filter(|n| n.role == Role::Trader).map(|n| n.stock.gold).sum();
    far(&mut w, 4.0 * 3600.0);
    let after = price_gap(&w);
    assert!(after.2 > 0, "herbs reached the east: {after:?}");
    assert!(after.3 < 60, "and left the west: {after:?}");
    let gold1: f32 = w.npcs.iter().filter(|n| n.role == Role::Trader).map(|n| n.stock.gold + 0.0).sum();
    let cargo: f32 = w.npcs.iter().filter(|n| n.role == Role::Trader).map(|n| n.stock.value(&w.economy().items)).sum();
    assert!(gold1 + cargo > gold0 * 0.9, "traders do not lose their shirts: {gold0} -> {gold1} + {cargo} cargo");
}

#[test]
fn an_active_npc_walks_to_the_vendor_and_trades_there() {
    let mut w = world(1, 31);
    w.nav = Some(Nav::from_tiles(vec![NavTile::flat(0, 0, 230.0, &[]), NavTile::flat(1024, 0, 230.0, &[])]));
    let i = w.npcs.iter().position(|n| n.role != Role::Vendor).unwrap();
    {
        let n = &mut w.npcs[i];
        n.role = Role::Adventurer;
        n.stock = Stock { gold: 100.0, items: Default::default() };
        n.stock.add("r_Iron", 4);
        n.pos.x = 400.0;
        n.pos.z = 10.0;
        n.at = 0;
        n.goal = Goal::Travel { dest: 0 };
    }
    let mut traded_at_post = false;
    for _ in 0..(15 * 240) {
        w.step(1.0 / 15.0, &[(0.0, 0.0)], 900.0); // a player watches from the town square: everybody here is active
        let n = &w.npcs[i];
        if let Goal::Trade { vendor, arrived: true, .. } = n.goal {
            let post = w.economy().vendors[vendor].post;
            let d = ((n.pos.x - post.0).powi(2) + (n.pos.z - post.1).powi(2)).sqrt();
            traded_at_post |= d < 30.0 && n.stock.count("r_Iron") == 0;
            assert!(n.lod == Lod::Active);
        }
    }
    assert!(traded_at_post, "it stood at a counter and had sold its iron");
}

#[test]
fn the_real_data_gives_a_world_with_shops() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let mut w = Npcs::load_with_economy(&format!("{root}/game-viewer/data/pois.json"), &format!("{root}/game-viewer/data/spawns.json"), &format!("{root}/game-viewer/data/items.json"), &format!("{root}/game-viewer/data/vendors.json"), 300, 1).unwrap();
    assert!(w.vendor_count() > 150, "vendors: {}", w.vendor_count());
    assert!(w.npcs.iter().filter(|n| n.role == Role::Trader).count() >= 8);
    let eco = w.economy();
    assert!(eco.items.list.len() > 500);
    // every shelf holds only goods of its trade
    for v in &eco.vendors {
        assert!(v.stock.items.keys().all(|k| v.kind.deals_in(eco.items.get(k).unwrap().group)), "{}", v.name);
    }
    drop(eco);
    far(&mut w, 3600.0);
    let eco = w.economy();
    for v in &eco.vendors {
        assert!(v.stock.gold.is_finite() && v.stock.gold >= 0.0, "{} has {} gold", v.name, v.stock.gold);
    }
    for n in &w.npcs {
        assert!(n.stock.gold.is_finite() && n.stock.gold >= -0.01, "{} has {} gold", n.info.name, n.stock.gold);
    }
}

#[tokio::test]
async fn vendors_are_in_the_roster_and_visible_near_their_post() {
    use game_server::proto::*;
    use game_server::world::{self, Cmd, Config, JoinReq};
    use tokio::sync::{mpsc, oneshot};
    use tokio_tungstenite::tungstenite::Message;

    let dir = std::env::temp_dir();
    let f = |n: &str| dir.join(format!("df_trade_{n}.json"));
    std::fs::write(f("pois"), r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(f("caps"), r#"[{"name":"Capital","race":"Human","x":100,"y":0,"z":200}]"#).unwrap();
    std::fs::write(f("items"), ITEMS).unwrap();
    // game z 200 -> wire z -200
    std::fs::write(f("vendors"), r#"[{"name":"Serawen","kind":"merchant","x":110,"z":200},{"name":"Targ","kind":"blacksmith","x":90,"z":205},{"name":"Far Away","kind":"merchant","x":9000,"z":9000}]"#).unwrap();
    let s = |n: &str| Some(f(n).to_string_lossy().to_string());
    let cfg = Config { radius: 900.0, max_players: 4, tick_hz: 30, spawns: None, weapons: None, pvp: false, npcs: 10, pois: s("pois"), capitals: s("caps"), npc_seed: 2, db: None, nav: None, npc_time_scale: 1.0, items: s("items"), vendors: s("vendors") };
    let (cmds, rx) = mpsc::channel(64);
    tokio::spawn(world::run(cfg, rx));
    let (tx, mut inbox) = mpsc::channel(4096);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "me".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: 105.0, y: 0.0, z: -200.0, flags: FLAG_FLY, ..Default::default() })).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
    let (mut vendors_in_roster, mut roles, mut seen_vendor) = (0, Vec::new(), false);
    while let Ok(m) = inbox.try_recv() {
        match m {
            Message::Text(t) if t.contains("\"welcome\"") => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                for p in v["players"].as_array().unwrap().iter().filter(|p| p["npc"] == true) {
                    roles.push(p["role"].as_str().unwrap().to_string());
                    if p["role"] == "vendor" {
                        vendors_in_roster += 1;
                        assert!(p["name"].as_str().unwrap().contains(" the "), "labelled by trade: {p}");
                    }
                }
            }
            Message::Binary(b) if b[0] == OP_SNAPSHOT => {
                let n = u16::from_le_bytes([b[5], b[6]]) as usize;
                for i in 0..n {
                    let o = 7 + i * 20;
                    let nid = u16::from_le_bytes([b[o], b[o + 1]]);
                    // vendors have the first ids
                    seen_vendor |= nid == NPC_ID_BASE || nid == NPC_ID_BASE + 1;
                }
            }
            _ => {}
        }
    }
    assert_eq!(vendors_in_roster, 2, "the vendor 9 km out of town is dropped: {roles:?}");
    assert_eq!(roles.len(), 12, "two vendors and ten others");
    assert!(seen_vendor, "the vendors stand in view of a player at the counter");
}
