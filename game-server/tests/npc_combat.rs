//! NPCs hunt for real when a player can see them: weapons come out, they swing at mobs, mobs hit back, and the player is told about all of it.
use std::collections::HashSet;
use std::time::Duration;

use game_server::npcs::NPC_ID_BASE;
use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const WEAPONS: &str = r#"{"weapons":{"r_Test Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":10},
    "r_Test Axe":{"group":"axe","kind":"melee","reach":2.2,"angle":60,"rank":12}},"clipDur":{"singlehand_attack_01":0.6}}"#;

async fn join(cmds: &mpsc::Sender<Cmd>, at: (f32, f32)) -> mpsc::Receiver<Message> {
    let (tx, rx) = mpsc::channel(8192);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "watcher".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: at.0, y: 0.0, z: at.1, flags: FLAG_FLY, ..Default::default() })).await.unwrap(); // flying: out of the fight
    rx
}

#[tokio::test]
async fn visible_npcs_hunt_mobs_and_mobs_hit_back() {
    let dir = std::env::temp_dir();
    let (pois, caps, spawns, weapons) = (dir.join("df_nc_pois.json"), dir.join("df_nc_caps.json"), dir.join("df_nc_spawns.json"), dir.join("df_nc_weapons.json"));
    // a village at game (100, 200) and a spawn of easy mobs 40 m away: wire z is the game z negated
    std::fs::write(&pois, r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(&caps, r#"[{"name":"Shoal","race":"Alfar","x":100,"y":0,"z":200}]"#).unwrap();
    std::fs::write(&spawns, r#"[{"id":1,"x":140,"z":200,"mobs":[{"id":7,"name":"Rat","qty":4,"difficulty":"Very Easy"},{"id":8,"name":"Wolf","qty":4,"difficulty":"Easy"}]}]"#).unwrap();
    std::fs::write(&weapons, WEAPONS).unwrap();
    let s = |p: &std::path::PathBuf| Some(p.to_string_lossy().to_string());
    let cfg = Config { radius: 900.0, max_players: 8, tick_hz: 60, spawns: s(&spawns), weapons: s(&weapons), pvp: false, npcs: 40, pois: s(&pois), capitals: s(&caps), npc_seed: 3, db: None, nav: None, npc_time_scale: 1.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    let mut watcher = join(&cmds, (120.0, -200.0)).await;

    let (mut drawn, mut swung, mut hit_mob, mut hit_npc, mut welcome_weapons) = (HashSet::new(), HashSet::new(), 0, 0, 0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline && !(hit_mob > 0 && hit_npc > 0) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        while let Ok(m) = watcher.try_recv() {
            let Message::Text(t) = m else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            match v["t"].as_str() {
                Some("welcome") => welcome_weapons = v["players"].as_array().unwrap().iter().filter(|p| p["npc"] == true && p["weapon"].as_str().map_or(false, |w| w.starts_with("r_Test"))).count(),
                Some("equip") if v["id"].as_u64().unwrap() >= NPC_ID_BASE as u64 && v["drawn"] == true => {
                    drawn.insert(v["id"].as_u64().unwrap());
                }
                Some("swing") if v["id"].as_u64().unwrap() >= NPC_ID_BASE as u64 => {
                    assert_eq!(v["clip"], "singlehand_attack_01");
                    swung.insert(v["id"].as_u64().unwrap());
                }
                Some("hit") if v["a"].as_u64().unwrap() >= NPC_ID_BASE as u64 && v["am"] == false && v["v"] == "m" => hit_mob += 1,
                Some("hit") if v["am"] == true && v["v"] == "p" && v["i"].as_u64().unwrap() >= NPC_ID_BASE as u64 => hit_npc += 1,
                _ => {}
            }
        }
    }
    assert_eq!(welcome_weapons, 40, "every npc is armed in the roster");
    assert!(!drawn.is_empty(), "an npc drew its weapon to hunt");
    assert!(!swung.is_empty(), "an npc swung at a mob");
    assert!(hit_mob > 0, "an npc's blow landed on a mob");
    assert!(hit_npc > 0, "a mob hit an npc back");
}

#[tokio::test]
async fn an_npc_killed_by_a_mob_leaves_a_corpse_and_gets_up_at_home() {
    let dir = std::env::temp_dir();
    let (pois, caps, spawns, weapons) = (dir.join("df_nd_pois.json"), dir.join("df_nd_caps.json"), dir.join("df_nd_spawns.json"), dir.join("df_nd_weapons.json"));
    // three bosses 12 m from the village centre: they kill whoever is within reach (NPCs on the move outrun them)
    std::fs::write(&pois, r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(&caps, r#"[{"name":"Shoal","race":"Alfar","x":100,"y":0,"z":200}]"#).unwrap();
    std::fs::write(&spawns, r#"[{"id":1,"x":112,"z":200,"mobs":[{"id":9,"name":"Boss","qty":3,"difficulty":"Boss"}]}]"#).unwrap();
    std::fs::write(&weapons, WEAPONS).unwrap();
    let s = |p: &std::path::PathBuf| Some(p.to_string_lossy().to_string());
    let cfg = Config { radius: 900.0, max_players: 8, tick_hz: 60, spawns: s(&spawns), weapons: s(&weapons), pvp: false, npcs: 150, pois: s(&pois), capitals: s(&caps), npc_seed: 3, db: None, nav: None, npc_time_scale: 1.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    let mut watcher = join(&cmds, (100.0, -200.0)).await;

    let (mut died, mut corpses, mut got_up) = (HashSet::new(), 0, HashSet::new());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(75);
    while tokio::time::Instant::now() < deadline && !died.iter().any(|d| got_up.contains(d)) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        while let Ok(m) = watcher.try_recv() {
            let Message::Text(t) = m else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            match v["t"].as_str() {
                Some("died") if v["v"] == "p" && v["i"].as_u64().unwrap() >= NPC_ID_BASE as u64 => {
                    died.insert(v["i"].as_u64().unwrap());
                }
                Some("corpse") => corpses += 1,
                Some("respawn") if v["id"].as_u64().unwrap() >= NPC_ID_BASE as u64 => {
                    got_up.insert(v["id"].as_u64().unwrap());
                }
                _ => {}
            }
        }
    }
    assert!(!died.is_empty(), "the boss killed an npc");
    assert!(corpses >= 1, "its body stays on the ground");
    assert!(died.iter().any(|d| got_up.contains(d)), "and it got up again: died {died:?}, got up {got_up:?}");
}
