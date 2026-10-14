//! Full loot at the NPC level: what an NPC drops when it dies, who goes for a tombstone, and the owner coming back for its things.
use server::combat::Weapons;
use server::economy::*;
use server::npcs::*;

const ITEMS: &str = r#"[{"id":"r_Apple","cost":2,"group":"food","stack":true},{"id":"r_Iron","cost":10,"group":"material","stack":true},
 {"id":"r_Iron Sword","cost":80,"group":"weapon"},{"id":"r_Steel Sword","cost":220,"group":"weapon"},{"id":"r_War Sword","cost":900,"group":"weapon"}]"#;
const WEAPONS: &str = r#"{"weapons":{"r_Iron Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":20},
 "r_Steel Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":35},"r_War Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":70}},"clipDur":{}}"#;

fn world(count: usize) -> Npcs {
    let places = vec![Place { name: "A".into(), x: 0.0, z: 0.0, town: true }, Place { name: "B".into(), x: 2000.0, z: 0.0, town: true }];
    let spots = [VendorSpot { name: "Ann".into(), kind: VendorKind::Merchant, x: 20.0, z: 5.0, }];
    let mut w = Npcs::build_with_economy(places, &[(0, (0.0, 0.0))], Items::parse(ITEMS).unwrap(), &spots, count, 9);
    w.equip_all(&Weapons::parse(WEAPONS).unwrap());
    w
}

/// The first adventurer, set up to do nothing but rest at (x, z) in town A.
fn adventurer(w: &mut Npcs, x: f32, z: f32) -> usize {
    let i = w.npcs.iter().position(|n| n.role == Role::Adventurer).unwrap();
    let n = &mut w.npcs[i];
    n.pos.x = x;
    n.pos.z = z;
    n.at = 0;
    n.goal = Goal::Rest { left: 600.0 };
    n.fatigue = 0.0;
    n.stock = Stock { gold: 70.0, items: [("r_Iron".to_string(), 3)].into() };
    n.info.weapon = "r_Iron Sword".into();
    n.group = "sword".into();
    i
}

fn far(w: &mut Npcs, seconds: f32, watcher: (f32, f32)) {
    for _ in 0..(seconds / 1.0) as usize {
        w.step(1.0, &[watcher], 10.0); // a tiny radius: nobody is active, so everything is settled abstractly
    }
}

#[test]
fn a_dying_npc_drops_everything_and_gets_up_with_a_plain_weapon() {
    let mut w = world(4);
    let i = adventurer(&mut w, 300.0, 0.0);
    let id = w.npcs[i].id;
    let dropped = w.drop_all(id);
    assert_eq!(dropped.gold, 70.0);
    assert_eq!(dropped.count("r_Iron"), 3);
    assert_eq!(dropped.count("r_Iron Sword"), 1, "the weapon in its hand too");
    let n = &w.npcs[i];
    assert!(n.stock.gold == 0.0 && n.stock.items.is_empty() && n.info.weapon.is_empty(), "nothing left on it");
    // it lies there for a while, then is back at home with the plainest sword there is
    w.npcs[i].down = 0.5;
    w.npcs[i].lod = Lod::Active;
    w.step(1.0, &[(300.0, 0.0)], 900.0);
    assert!(w.revived.contains(&id));
    let n = &w.npcs[i];
    assert_eq!(n.info.weapon, "r_Iron Sword", "the lowest-rank sword");
    assert!(n.hp > 0.0 && n.stock.gold == 0.0);
    assert!(w.weapon_changed.borrow().contains(&id));
}

#[test]
fn an_npc_that_sees_a_tombstone_goes_and_takes_what_is_on_it() {
    let mut w = world(4);
    let i = adventurer(&mut w, 300.0, 0.0);
    // 40 m away, a body with gold, goods and a better sword than the one in hand
    let loot = Stock { gold: 55.0, items: [("r_Apple".to_string(), 2), ("r_Steel Sword".to_string(), 1)].into() };
    w.add_grave(77, 340.0, 0.0, loot, None);
    assert!(w.grave(77).is_some());
    far(&mut w, 120.0, (-50_000.0, 0.0));
    assert!(w.grave(77).is_none(), "emptied");
    assert_eq!(w.looted.borrow().as_slice(), &[77]);
    let n = &w.npcs[i];
    assert!(n.stock.gold >= 70.0 + 55.0 - 10.0, "the 55 gold came with it (it may have spent a little on meals): {}", n.stock.gold);
    assert_eq!(n.info.weapon, "r_Steel Sword", "and it took the better sword into its hand");
    assert!(n.stock.count("r_Apple") >= 2 && n.stock.count("r_Iron Sword") == 1, "the old sword is carried: {:?}", n.stock.items);
}

#[test]
fn nobody_goes_for_a_tombstone_out_of_sight_and_an_empty_one_is_ignored() {
    let mut w = world(4);
    let i = adventurer(&mut w, 300.0, 0.0);
    w.add_grave(1, 900.0, 0.0, Stock { gold: 10.0, items: Default::default() }, None); // 600 m away
    w.add_grave(2, 320.0, 0.0, Stock::default(), None); // nothing on it
    far(&mut w, 60.0, (-50_000.0, 0.0));
    assert!(w.grave(1).is_some(), "too far to notice");
    assert!(w.grave(2).is_none(), "an empty tombstone is not a grave worth a walk");
    assert_eq!(w.npcs[i].stock.gold, 70.0);
    assert!(w.looted.borrow().is_empty());
    // removing it when it rots
    w.remove_grave(1);
    assert!(w.grave(1).is_none());
}

#[test]
fn first_come_first_served_and_players_can_take_from_graves_too() {
    let mut w = world(4);
    w.add_grave(5, 5000.0, 0.0, Stock { gold: 40.0, items: [("r_Iron".to_string(), 2)].into() }, None);
    let a = w.take_grave(5).expect("the first one gets it");
    assert_eq!((a.gold, a.count("r_Iron")), (40.0, 2));
    assert!(w.take_grave(5).is_none(), "and the second finds it empty");
    assert!(w.grave(5).is_none());
}

#[test]
fn the_owner_goes_back_for_its_things() {
    let mut w = world(4);
    let i = adventurer(&mut w, 100.0, 0.0);
    let id = w.npcs[i].id;
    // it died 1.2 km from home with 70 gold and a sword
    let dropped = w.drop_all(id);
    w.add_grave(9, 1200.0, 0.0, dropped, Some(id));
    w.npcs[i].down = 0.2;
    w.npcs[i].lod = Lod::Active;
    w.step(1.0, &[(5000.0, 5000.0)], 900.0); // revives at home (town A)
    assert!(w.npcs[i].down == 0.0);
    let mut got_it = false;
    for _ in 0..(40 * 60) {
        w.step(1.0, &[(-50_000.0, 0.0)], 10.0);
        if w.grave(9).is_none() {
            got_it = true;
            break;
        }
    }
    assert!(got_it, "it went back and took its things");
    let n = &w.npcs[i];
    assert!(n.stock.gold >= 70.0, "{}", n.stock.gold);
    assert!(n.info.weapon == "r_Iron Sword" || n.stock.count("r_Iron Sword") > 0, "its sword is back: {} / {:?}", n.info.weapon, n.stock.items);
}

/// A village with three bosses 12 m from its centre: the villagers wander in and die (see npc_combat.rs). Bodies turn into tombstones after 1.5 s,
/// a loot channel takes 0.6 s.
fn deadly_village(tag: &str, npcs: usize) -> mpsc::Sender<server::world::Cmd> {
    use server::world::{self, Config};
    let dir = std::env::temp_dir();
    let f = |n: &str| dir.join(format!("df_lootw_{tag}_{n}.json"));
    std::fs::write(f("pois"), r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(f("caps"), r#"[{"name":"Capital","race":"Human","x":100,"y":0,"z":200}]"#).unwrap();
    std::fs::write(f("spawns"), r#"[{"id":1,"x":112,"z":200,"mobs":[{"id":9,"name":"Boss","qty":3,"difficulty":"Boss"}]}]"#).unwrap();
    let s = |n: &str| Some(f(n).to_string_lossy().to_string());
    let cfg = Config { max_speed: 0.0, max_fly_speed: 0.0, ghost_secs: 300.0, logout_secs: 30.0, radius: 900.0, max_players: 4, tick_hz: 60, spawns: s("spawns"), weapons: None, pvp: false, npcs, pois: s("pois"), capitals: s("caps"), npc_seed: 3, db: None, nav: None, npc_time_scale: 1.0, own_weapons: false, body_secs: 1.5, tomb_secs: 3600, loot_secs: 0.6, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    cmds
}

async fn visitor(cmds: &mpsc::Sender<server::world::Cmd>) -> (u16, mpsc::Receiver<tokio_tungstenite::tungstenite::Message>) {
    use server::proto::*;
    use server::world::{Cmd, JoinReq};
    let (tx, inbox) = mpsc::channel(32768);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { account: None, name: "visitor".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let me = reply_rx.await.unwrap().unwrap();
    // watching from the edge of the village, out of the fight (free-fly)
    cmds.send(Cmd::Pos(me, PosUpdate { x: 100.0, y: 0.0, z: -230.0, flags: FLAG_FLY, ..Default::default() })).await.unwrap();
    (me, inbox)
}

use server::world::Cmd;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn a_body_turns_into_a_tombstone_that_a_player_takes_in_a_channel_and_a_dying_player_drops_everything() {
    use server::proto::*;
    let cmds = deadly_village("full", 150);
    let (me, mut inbox) = visitor(&cmds).await;

    let (mut bodies, mut tombs) = (std::collections::HashMap::<u64, std::time::Instant>::new(), Vec::new());
    let (mut my_gold, mut looting_started, mut loot_ok, mut looted_msgs) = (0.0f64, false, false, Vec::new());
    let (mut boss, mut my_corpse, mut my_tomb, mut equipped) = (None::<(f32, f32)>, false, None::<(f64, u64)>, false);
    let mut early_loot_got_looting = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    while tokio::time::Instant::now() < deadline && my_tomb.is_none() {
        tokio::time::sleep(Duration::from_millis(80)).await;
        while let Ok(m) = inbox.try_recv() {
            if let Message::Binary(b) = &m {
                if b[0] == OP_MOBS && u16::from_le_bytes([b[5], b[6]]) > 0 {
                    let f = |k: usize| f32::from_le_bytes([b[7 + k], b[8 + k], b[9 + k], b[10 + k]]);
                    boss = Some((f(4), f(12)));
                }
                continue;
            }
            let Message::Text(t) = m else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            match v["t"].as_str() {
                Some("corpse") if v["name"] != "visitor" => {
                    bodies.insert(v["cid"].as_u64().unwrap(), std::time::Instant::now());
                }
                Some("corpse") => my_corpse = true,
                Some("tombstone") if v["name"] != "visitor" => {
                    let cid = v["cid"].as_u64().unwrap();
                    if let Some(at) = bodies.get(&cid) {
                        assert!(at.elapsed() > Duration::from_millis(1200), "the body lay there for its 1.5 s first");
                    }
                    tombs.push((cid, v["x"].as_f64().unwrap() as f32, v["z"].as_f64().unwrap() as f32, v["gold"].as_f64().unwrap()));
                }
                Some("tombstone") => my_tomb = Some((v["gold"].as_f64().unwrap(), v["units"].as_u64().unwrap())),
                Some("looting") => looting_started = true,
                Some("lootend") => loot_ok = v["ok"] == true,
                Some("looted") => looted_msgs.push(v["cid"].as_u64().unwrap()),
                Some("stock") => {
                    let g = v["gold"].as_f64().unwrap();
                    if g > my_gold && loot_ok {
                        my_gold = g;
                    }
                }
                _ => {}
            }
        }
        if my_gold == 0.0 {
            // the first tombstone nobody has emptied: stand by it and take it
            if let Some(&(cid, x, z, _)) = tombs.iter().find(|t| !looted_msgs.contains(&t.0)) {
                if !early_loot_got_looting {
                    // a body that is still lying there (not yet a tombstone) cannot be looted: nothing starts
                    early_loot_got_looting = true;
                    if let Some((&bcid, _)) = bodies.iter().find(|(k, _)| !tombs.iter().any(|t| t.0 == **k)) {
                        cmds.send(Cmd::Loot(me, bcid as u32)).await.unwrap();
                    }
                }
                looting_started = false;
                cmds.send(Cmd::Pos(me, PosUpdate { x, y: 0.0, z, flags: FLAG_FLY, ..Default::default() })).await.unwrap();
                cmds.send(Cmd::Loot(me, cid as u32)).await.unwrap();
                tokio::time::sleep(Duration::from_millis(1100)).await; // the 0.6 s channel
            }
        } else if !equipped {
            // wields a sword (so there is a weapon to drop), then walks into the bosses
            cmds.send(Cmd::Equip(me, "r_Test Sword".into(), true)).await.unwrap();
            equipped = true;
        } else {
            let (bx, bz) = boss.unwrap_or((112.0, -200.0));
            cmds.send(Cmd::Pos(me, PosUpdate { x: bx + 0.5, y: 0.0, z: bz, ..Default::default() })).await.unwrap();
        }
    }
    assert!(looting_started || my_gold > 0.0, "the channel started");
    assert!(my_gold > 0.0, "the player took the gold of a tombstone");
    assert!(my_corpse, "the visitor died and left a body");
    let (dropped_gold, units) = my_tomb.expect("and the body turned into a tombstone");
    assert!((dropped_gold - my_gold).abs() < 0.01, "he dropped all he carried: {dropped_gold} of {my_gold}");
    assert!(units >= 1, "his sword too: {units}");
}

#[tokio::test]
async fn nothing_can_be_taken_from_a_body_and_an_empty_handed_one_just_goes() {
    use server::proto::*;
    let cmds = deadly_village("empty", 0);
    let (me, mut inbox) = visitor(&cmds).await;
    // no villagers: only the visitor can die. He carries nothing and wields nothing.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mut boss = None;
    while let Ok(m) = inbox.try_recv() {
        if let Message::Binary(b) = &m {
            if b[0] == OP_MOBS && u16::from_le_bytes([b[5], b[6]]) > 0 {
                let f = |k: usize| f32::from_le_bytes([b[7 + k], b[8 + k], b[9 + k], b[10 + k]]);
                boss = Some((f(4), f(12)));
            }
        }
    }
    let (bx, bz) = boss.expect("a mob frame");
    let (mut corpse, mut tomb, mut gone, mut looting) = (None::<(u64, std::time::Instant)>, false, false, false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while tokio::time::Instant::now() < deadline && !gone {
        tokio::time::sleep(Duration::from_millis(80)).await;
        if corpse.is_none() {
            cmds.send(Cmd::Pos(me, PosUpdate { x: bx + 0.5, y: 0.0, z: bz, ..Default::default() })).await.unwrap();
        } else {
            // standing at the body: it cannot be looted
            cmds.send(Cmd::Loot(me, corpse.unwrap().0 as u32)).await.unwrap();
        }
        while let Ok(m) = inbox.try_recv() {
            let Message::Text(t) = m else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            match v["t"].as_str() {
                Some("corpse") if v["name"] == "visitor" => {
                    corpse = Some((v["cid"].as_u64().unwrap(), std::time::Instant::now()));
                    // the dead rise after 6 s, but the body is what matters here; stay down by not moving
                }
                Some("tombstone") => tomb = true,
                Some("looting") => looting = true,
                Some("corpsegone") if corpse.map_or(false, |c| v["cid"].as_u64() == Some(c.0)) => {
                    assert!(corpse.unwrap().1.elapsed() > Duration::from_millis(1200), "it lay there its 1.5 s");
                    gone = true;
                }
                _ => {}
            }
        }
    }
    assert!(gone, "the empty-handed body went away");
    assert!(!tomb, "with no tombstone");
    assert!(!looting, "and there was nothing to loot meanwhile");
}

#[tokio::test]
async fn a_loot_channel_is_interrupted_by_walking_away() {
    use server::proto::*;
    let cmds = deadly_village("walk", 150);
    let (me, mut inbox) = visitor(&cmds).await;
    let (mut tomb, mut started, mut ended) = (None::<(u64, f32, f32)>, false, None::<bool>);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    while tokio::time::Instant::now() < deadline && ended.is_none() {
        tokio::time::sleep(Duration::from_millis(60)).await;
        while let Ok(m) = inbox.try_recv() {
            let Message::Text(t) = m else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            match v["t"].as_str() {
                Some("tombstone") if tomb.is_none() => tomb = Some((v["cid"].as_u64().unwrap(), v["x"].as_f64().unwrap() as f32, v["z"].as_f64().unwrap() as f32)),
                Some("looting") => started = true,
                Some("lootend") if started => ended = Some(v["ok"] == true),
                _ => {}
            }
        }
        if let (Some((cid, x, z)), false) = (tomb, started) {
            cmds.send(Cmd::Pos(me, PosUpdate { x, y: 0.0, z, flags: FLAG_FLY, ..Default::default() })).await.unwrap();
            cmds.send(Cmd::Loot(me, cid as u32)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            // a step away, half way through
            cmds.send(Cmd::Pos(me, PosUpdate { x: x + 5.0, y: 0.0, z, flags: FLAG_FLY, ..Default::default() })).await.unwrap();
        }
    }
    assert_eq!(ended, Some(false), "walking away interrupted the channel");
}

#[test]
fn an_npc_that_is_hurt_while_looting_is_interrupted() {
    let mut w = world(4);
    let i = adventurer(&mut w, 300.0, 0.0);
    w.add_grave(31, 330.0, 0.0, Stock { gold: 33.0, items: Default::default() }, None);
    // a player watches: the NPC is active and stepped in real time. It walks to the body and the channel starts.
    let mut at_body = false;
    for _ in 0..(15 * 30) {
        w.step(1.0 / 15.0, &[(300.0, 0.0)], 900.0);
        if matches!(w.npcs[i].goal, Goal::Loot { arrived: true, .. }) {
            at_body = true;
            break;
        }
    }
    assert!(at_body, "it walked to the body and started taking it");
    assert!(w.grave(31).is_some(), "the contents are only taken at the end of the channel");
    w.npcs[i].hp -= 5.0; // something hits it
    w.step(1.0 / 15.0, &[(300.0, 0.0)], 900.0);
    assert!(!matches!(w.npcs[i].goal, Goal::Loot { arrived: true, .. }), "the channel stopped");
    assert!(w.grave(31).is_some(), "interrupted: the body is untouched");
    // left alone it goes back to it and finishes
    let mut taken = false;
    for _ in 0..(15 * 60) {
        w.step(1.0 / 15.0, &[(300.0, 0.0)], 900.0);
        if w.grave(31).is_none() {
            taken = true;
            break;
        }
    }
    assert!(taken, "and takes it the next time");
}

#[test]
fn a_tombstone_gives_only_what_fits_and_keeps_the_rest() {
    let w = world(1);
    let items = Items::parse(ITEMS).unwrap();
    // 100 iron (weight 1 each in the fixture catalogue? the fixture has no weights: the group fallback, 1 per unit) and 40 gold
    w.add_grave_for_test(7, Stock { gold: 40.0, items: [("r_Iron".to_string(), 100)].into() });
    let free = 7.5;
    let taken = w.take_grave_fitting(7, free, &items).expect("something fits");
    assert_eq!(taken.gold, 40.0, "the gold weighs nothing: all of it");
    assert_eq!(taken.count("r_Iron"), 7, "7 units of weight 1 in 7.5");
    assert!(w.grave(7).is_some(), "the rest stays on the tombstone");
    assert_eq!(w.grave(7).unwrap().stock.count("r_Iron"), 93);
    assert_eq!(w.grave(7).unwrap().stock.gold, 0.0);
    assert!(w.looted.borrow().is_empty());
    // nothing fits at all: nothing taken, nothing lost
    assert!(w.take_grave_fitting(7, 0.4, &items).is_none());
    assert_eq!(w.grave(7).unwrap().stock.count("r_Iron"), 93);
    // room enough: the rest, and now it is empty and gone
    let taken = w.take_grave_fitting(7, 500.0, &items).unwrap();
    assert_eq!(taken.count("r_Iron"), 93);
    assert!(w.grave(7).is_none());
    assert_eq!(w.looted.borrow().as_slice(), &[7]);
}

#[test]
fn single_items_and_gold_can_be_taken_one_at_a_time_and_the_tombstone_goes_when_empty() {
    let mut w = world(0);
    let items = Items::parse(ITEMS).unwrap();
    w.add_grave(5, 0.0, 0.0, Stock { gold: 40.0, items: [("r_Apple".to_string(), 3), ("r_Iron".to_string(), 2)].into() }, None);
    let t = w.take_grave_item(5, "r_Apple", 2, 1000.0, &items).unwrap();
    assert_eq!(t.count("r_Apple"), 2);
    assert_eq!(w.grave(5).unwrap().stock.count("r_Apple"), 1);
    let g = w.take_grave_item(5, "", 1, 0.0, &items).unwrap();
    assert_eq!(g.gold, 40.0);
    w.take_grave_item(5, "r_Apple", 9, 1000.0, &items).unwrap();
    w.take_grave_item(5, "r_Iron", 9, 1000.0, &items).unwrap();
    assert!(w.grave(5).is_none() && w.looted.borrow().contains(&5));
}
