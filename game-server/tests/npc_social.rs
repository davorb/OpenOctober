//! How NPCs treat players: the chaos races attack the others (with pvp on), friendly ones greet, and anybody fights back against a player who hurt it.
use std::collections::HashSet;
use std::time::Duration;

use game_server::npcs::NPC_ID_BASE;
use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const WEAPONS: &str = r#"{"weapons":{"r_Test Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":10}},"clipDur":{"singlehand_attack_01":0.6}}"#;

struct Setup {
    cmds: mpsc::Sender<Cmd>,
}

/// A village at game (100, 200) whose capital belongs to `race`; 30 NPCs of that race live there.
fn start(tag: &str, race: &str, pvp: bool) -> Setup {
    let dir = std::env::temp_dir();
    let f = |n: &str| dir.join(format!("df_soc_{tag}_{n}.json"));
    std::fs::write(f("pois"), r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(f("caps"), format!(r#"[{{"name":"Capital","race":"{race}","x":100,"y":0,"z":200}}]"#)).unwrap();
    std::fs::write(f("weapons"), WEAPONS).unwrap();
    let s = |n: &str| Some(f(n).to_string_lossy().to_string());
    let cfg = Config { radius: 900.0, max_players: 8, tick_hz: 60, spawns: None, weapons: s("weapons"), pvp, npcs: 30, pois: s("pois"), capitals: s("caps"), npc_seed: 5, db: None, nav: None, npc_time_scale: 1.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    Setup { cmds }
}

async fn join(cmds: &mpsc::Sender<Cmd>, race: &str, at: (f32, f32)) -> (u16, mpsc::Receiver<Message>) {
    let (tx, rx) = mpsc::channel(16384);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "visitor".into(), race: race.into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: at.0, y: 0.0, z: at.1, ..Default::default() })).await.unwrap();
    (id, rx)
}

fn json(m: Message) -> Option<serde_json::Value> {
    match m {
        Message::Text(t) => serde_json::from_str(&t).ok(),
        _ => None,
    }
}

#[tokio::test]
async fn hostile_npcs_attack_players_of_the_other_side() {
    let s = start("hostile", "Ork", true);
    let (me, mut rx) = join(&s.cmds, "hum", (110.0, -200.0)).await;
    let (mut swings, mut hits) = (HashSet::new(), 0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while tokio::time::Instant::now() < deadline && hits == 0 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        while let Ok(m) = rx.try_recv() {
            let Some(v) = json(m) else { continue };
            match v["t"].as_str() {
                Some("swing") if v["id"].as_u64().unwrap() >= NPC_ID_BASE as u64 => {
                    swings.insert(v["id"].as_u64().unwrap());
                }
                Some("hit") if v["v"] == "p" && v["i"].as_u64() == Some(me as u64) && v["a"].as_u64().unwrap() >= NPC_ID_BASE as u64 && v["am"] == false => hits += 1,
                _ => {}
            }
        }
    }
    assert!(!swings.is_empty(), "an orc swung at the human");
    assert!(hits > 0, "and hit him");
}

#[tokio::test]
async fn no_hostility_without_pvp_and_none_between_friends() {
    for (tag, npc_race, pvp) in [("nopvp", "Ork", false), ("friends", "Human", true)] {
        let s = start(tag, npc_race, pvp);
        let (me, mut rx) = join(&s.cmds, "hum", (110.0, -200.0)).await;
        let mut greeted = HashSet::new();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(200)).await;
            while let Ok(m) = rx.try_recv() {
                let Some(v) = json(m) else { continue };
                match v["t"].as_str() {
                    Some("hit") => assert!(v["i"].as_u64() != Some(me as u64), "{tag}: nobody hits the visitor: {v}"),
                    Some("swing") => assert!(v["id"].as_u64().unwrap() < NPC_ID_BASE as u64, "{tag}: no npc swings"),
                    Some("chat") if v["id"].as_u64().unwrap() >= NPC_ID_BASE as u64 => {
                        greeted.insert(v["id"].as_u64().unwrap());
                    }
                    _ => {}
                }
            }
        }
        if tag == "friends" {
            assert!(!greeted.is_empty(), "a friendly npc greets a player of its own side");
        } else {
            assert!(greeted.is_empty(), "orcs do not greet a human");
        }
    }
}

#[tokio::test]
async fn a_player_who_hurts_an_npc_is_fought_back() {
    // friendly (human) NPCs in a village; the visitor is human too, so nobody minds him until he strikes
    let s = start("grudge", "Human", true);
    let (me, mut rx) = join(&s.cmds, "hum", (110.0, -200.0)).await;
    s.cmds.send(Cmd::Equip(me, "r_Test Sword".into(), true)).await.unwrap();
    let (mut struck, mut retaliated) = (false, false);
    let mut target: Option<(u16, f32, f32)> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline && !retaliated {
        tokio::time::sleep(Duration::from_millis(100)).await;
        while let Ok(m) = rx.try_recv() {
            match m {
                Message::Binary(b) if b[0] == OP_SNAPSHOT => {
                    let n = u16::from_le_bytes([b[5], b[6]]) as usize;
                    for i in 0..n {
                        let o = 7 + i * 20;
                        let id = u16::from_le_bytes([b[o], b[o + 1]]);
                        let f = |k: usize| f32::from_le_bytes([b[o + k], b[o + k + 1], b[o + k + 2], b[o + k + 3]]);
                        if id >= NPC_ID_BASE && target.map_or(true, |t| t.0 == id) {
                            target = Some((id, f(2), f(10)));
                        }
                    }
                }
                Message::Text(t) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    match v["t"].as_str() {
                        Some("hit") if v["a"].as_u64() == Some(me as u64) && v["v"] == "p" && v["i"].as_u64().unwrap() >= NPC_ID_BASE as u64 => struck = true,
                        Some("hit") if v["i"].as_u64() == Some(me as u64) && v["a"].as_u64().unwrap() >= NPC_ID_BASE as u64 => retaliated = true,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if let Some((_, x, z)) = target {
            // stand 2 m south of it, facing it (yaw 0 looks along -z), and swing
            s.cmds.send(Cmd::Pos(me, PosUpdate { x, y: 0.0, z: z + 2.0, yaw: 0.0, ..Default::default() })).await.unwrap();
            if !struck {
                s.cmds.send(Cmd::Attack(me, "melee".into(), "singlehand_attack_01".into())).await.unwrap();
            }
        }
    }
    assert!(struck, "the visitor's blow landed on an npc");
    assert!(retaliated, "and the npc hit him back");
}
