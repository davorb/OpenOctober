//! Full loot at the NPC level: what an NPC drops when it dies, who goes for a tombstone, and the owner coming back for its things.
use game_server::combat::Weapons;
use game_server::economy::*;
use game_server::npcs::*;

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

#[tokio::test]
async fn a_player_takes_a_tombstone_then_dies_and_drops_what_he_carried() {
    use game_server::proto::*;
    use game_server::world::{self, Cmd, Config, JoinReq};
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot};
    use tokio_tungstenite::tungstenite::Message;

    let dir = std::env::temp_dir();
    let f = |n: &str| dir.join(format!("df_lootw_{n}.json"));
    // a village with three bosses 12 m from its centre: the villagers wander in and die (see npc_combat.rs)
    std::fs::write(f("pois"), r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(f("caps"), r#"[{"name":"Capital","race":"Human","x":100,"y":0,"z":200}]"#).unwrap();
    std::fs::write(f("spawns"), r#"[{"id":1,"x":112,"z":200,"mobs":[{"id":9,"name":"Boss","qty":3,"difficulty":"Boss"}]}]"#).unwrap();
    let s = |n: &str| Some(f(n).to_string_lossy().to_string());
    let cfg = Config { radius: 900.0, max_players: 4, tick_hz: 60, spawns: s("spawns"), weapons: None, pvp: false, npcs: 150, pois: s("pois"), capitals: s("caps"), npc_seed: 3, db: None, nav: None, npc_time_scale: 1.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    let (tx, mut inbox) = mpsc::channel(32768);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "visitor".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let me = reply_rx.await.unwrap().unwrap();
    // watching from the edge of the village, out of the fight (free-fly)
    cmds.send(Cmd::Pos(me, PosUpdate { x: 100.0, y: 0.0, z: -230.0, flags: FLAG_FLY, ..Default::default() })).await.unwrap();

    let (mut bodies, mut my_gold, mut taken_cid, mut looted_msgs) = (Vec::new(), 0.0f64, None::<u64>, Vec::new());
    let (mut dropped_gold, mut stock_after_death) = (None::<f64>, None::<f64>);
    let mut boss: Option<(f32, f32)> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while tokio::time::Instant::now() < deadline && dropped_gold.is_none() {
        tokio::time::sleep(Duration::from_millis(100)).await;
        while let Ok(m) = inbox.try_recv() {
            if let Message::Binary(b) = &m {
                if b[0] == OP_MOBS && u16::from_le_bytes([b[5], b[6]]) > 0 {
                    // the first mob of the frame (a boss: the only mobs there are)
                    let f = |k: usize| f32::from_le_bytes([b[7 + k], b[8 + k], b[9 + k], b[10 + k]]);
                    boss = Some((f(4), f(12)));
                }
                continue;
            }
            let Message::Text(t) = m else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            match v["t"].as_str() {
                Some("corpse") if v["name"] != "visitor" && v["gold"].as_f64().unwrap_or(0.0) > 0.0 => bodies.push((v["cid"].as_u64().unwrap(), v["x"].as_f64().unwrap() as f32, v["z"].as_f64().unwrap() as f32, v["gold"].as_f64().unwrap())),
                Some("corpse") if v["name"] == "visitor" => dropped_gold = Some(v["gold"].as_f64().unwrap()),
                Some("looted") => looted_msgs.push(v["cid"].as_u64().unwrap()),
                Some("stock") => {
                    let g = v["gold"].as_f64().unwrap();
                    if taken_cid.is_some() && g > my_gold {
                        my_gold = g;
                    }
                    if dropped_gold.is_some() || (my_gold > 0.0 && g == 0.0) {
                        stock_after_death = Some(g);
                    }
                }
                _ => {}
            }
        }
        // step 1: run to the first untouched body and take it (before any villager can walk over)
        if taken_cid.is_none() {
            if let Some(&(cid, x, z, _)) = bodies.iter().find(|b| !looted_msgs.contains(&b.0)) {
                cmds.send(Cmd::Pos(me, PosUpdate { x, y: 0.0, z, flags: FLAG_FLY, ..Default::default() })).await.unwrap();
                cmds.send(Cmd::Loot(me, cid as u32)).await.unwrap();
                tokio::time::sleep(Duration::from_millis(250)).await;
                taken_cid = Some(cid);
                // if a villager beat us to it, try the next body
                continue;
            }
        } else if my_gold == 0.0 {
            if looted_msgs.contains(&taken_cid.unwrap()) {
                taken_cid = None; // somebody else emptied it: next one
                bodies.retain(|b| b.0 != taken_cid.unwrap_or(0));
            }
        } else {
            // step 2: walk into the bosses, out of free-fly: he dies and drops what he carries
            let (bx, bz) = boss.unwrap_or((112.0, -200.0));
            cmds.send(Cmd::Pos(me, PosUpdate { x: bx + 0.5, y: 0.0, z: bz, ..Default::default() })).await.unwrap();
        }
    }
    assert!(my_gold > 0.0, "the player took the gold of a tombstone");
    let taken = taken_cid.unwrap();
    assert!(looted_msgs.contains(&taken), "everybody was told it was emptied");
    let dropped = dropped_gold.expect("the visitor died and left a body");
    assert!((dropped - my_gold).abs() < 0.01, "he dropped all he carried: {dropped} of {my_gold}");
    assert_eq!(stock_after_death, Some(0.0), "and was told he carries nothing now");
}
