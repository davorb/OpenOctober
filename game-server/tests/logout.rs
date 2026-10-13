//! Lost connections and `/logout`: a body whose connection dropped stays standing (killable, lootable once dead); `/logout` is a 30 s wait with
//! everybody hearing it, cancelled by moving; the owner who comes back takes his body over.
use std::time::Duration;

use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const WEAPONS: &str = r#"{"weapons":{"r_Test Doom":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":400}},"clipDur":{"singlehand_attack_01":1.0}}"#;

struct Client {
    id: u16,
    rx: mpsc::Receiver<Message>,
}

async fn start(ghost_secs: f32, logout_secs: f32) -> mpsc::Sender<Cmd> {
    let w = std::env::temp_dir().join("df_logout_w.json");
    std::fs::write(&w, WEAPONS).unwrap();
    let cfg = Config { max_speed: 0.0, max_fly_speed: 0.0, ghost_secs, logout_secs, radius: 900.0, max_players: 8, tick_hz: 30, spawns: None, weapons: Some(w.to_string_lossy().into()), pvp: true, npcs: 0, pois: None, capitals: None, npc_seed: 1, db: None, nav: None, npc_time_scale: 1.0, own_weapons: false, body_secs: 45.0, tomb_secs: 3600, loot_secs: 3.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    cmds
}

async fn join(cmds: &mpsc::Sender<Cmd>, account: Option<i64>, at: (f32, f32)) -> Client {
    let (tx, rx) = mpsc::channel(4096);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { account, name: "t".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: at.0, y: 0.0, z: at.1, ..Default::default() })).await.unwrap();
    Client { id, rx }
}

fn texts(c: &mut Client) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(m) = c.rx.try_recv() {
        if let Message::Text(t) = m {
            out.push(t);
        }
    }
    out
}

/// Ids in the latest position snapshot this client got.
fn visible(c: &mut Client) -> Vec<u16> {
    let mut last = Vec::new();
    while let Ok(m) = c.rx.try_recv() {
        if let Message::Binary(b) = m {
            if b[0] == OP_SNAPSHOT {
                let n = u16::from_le_bytes([b[5], b[6]]) as usize;
                last = (0..n).map(|i| u16::from_le_bytes([b[7 + i * 20], b[8 + i * 20]])).collect();
            }
        }
    }
    last
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test]
async fn a_dropped_connection_leaves_a_body_that_can_be_killed_and_is_gone_after_the_wait() {
    let cmds = start(1.0, 30.0).await;
    let mut a = join(&cmds, None, (300.0, -300.0)).await;
    let b = join(&cmds, Some(7), (300.0, -302.5)).await;
    cmds.send(Cmd::Leave(b.id)).await.unwrap();
    drop(b.rx);
    pause(300).await;
    let seen = texts(&mut a);
    assert!(!seen.iter().any(|t| t.contains("\"t\":\"left\"")), "the body must stay: {seen:?}");
    pause(150).await;
    assert!(visible(&mut a).contains(&b.id), "the body is still in the snapshots");
    pause(1000).await;
    assert!(texts(&mut a).iter().any(|t| t.contains("\"t\":\"left\"")), "after the wait it is gone");
    assert!(!visible(&mut a).contains(&b.id));
}

#[tokio::test]
async fn a_body_without_its_owner_dies_and_drops_a_corpse_and_the_owner_gets_up_at_the_capital() {
    let cmds = start(300.0, 30.0).await;
    let mut a = join(&cmds, None, (300.0, -300.0)).await;
    let b = join(&cmds, Some(7), (300.0, -302.5)).await;
    cmds.send(Cmd::Leave(b.id)).await.unwrap();
    drop(b.rx);
    cmds.send(Cmd::Equip(a.id, "r_Test Doom".into(), true)).await.unwrap();
    pause(100).await;
    texts(&mut a);
    // a is facing +z (yaw 0 looks along -z in the viewer, as the combat tests do): swing towards b
    cmds.send(Cmd::Pos(a.id, PosUpdate { x: 300.0, y: 0.0, z: -300.0, yaw: 0.0, ..Default::default() })).await.unwrap();
    cmds.send(Cmd::Attack(a.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    pause(900).await;
    let seen = texts(&mut a);
    assert!(seen.iter().any(|t| t.contains("\"t\":\"corpse\"")), "killed body leaves a corpse: {seen:?}");
    assert!(seen.iter().any(|t| t.contains("\"t\":\"left\"")), "the dead ghost is out of the roster");
    // he logs in again: not in his old body, he gets up
    let mut b2 = join(&cmds, Some(7), (0.0, 0.0)).await;
    pause(150).await;
    let t = texts(&mut b2);
    assert!(t.iter().any(|t| t.contains("\"t\":\"respawn\"")), "{t:?}");
}

#[tokio::test]
async fn the_owner_takes_his_standing_body_over() {
    let cmds = start(300.0, 30.0).await;
    let mut a = join(&cmds, None, (300.0, -300.0)).await;
    let b = join(&cmds, Some(7), (300.0, -310.0)).await;
    let old = b.id;
    cmds.send(Cmd::Leave(b.id)).await.unwrap();
    drop(b.rx);
    pause(100).await;
    texts(&mut a);
    let mut b2 = join(&cmds, Some(7), (300.0, -310.0)).await;
    assert_eq!(b2.id, old, "same body, same id");
    pause(200).await;
    let t = texts(&mut b2);
    assert!(t.iter().any(|t| t.contains("\"t\":\"welcome\"")));
    assert!(!t.iter().any(|t| t.contains("\"t\":\"respawn\"")));
    assert!(!texts(&mut a).iter().any(|t| t.contains("\"t\":\"left\"") || t.contains("\"t\":\"joined\"")), "the others saw nothing change");
}

#[tokio::test]
async fn logout_is_announced_cancelled_by_moving_and_completes() {
    let cmds = start(300.0, 1.0).await;
    let mut a = join(&cmds, None, (300.0, -300.0)).await;
    let mut b = join(&cmds, Some(1), (400.0, -300.0)).await;
    pause(100).await;
    texts(&mut a);
    texts(&mut b);
    cmds.send(Cmd::Chat(b.id, "/logout".into())).await.unwrap();
    pause(100).await;
    assert!(texts(&mut a).iter().any(|t| t.contains("\"t\":\"logout\"") && t.contains(&format!("\"id\":{}", b.id))));
    // he cannot swing meanwhile: nothing is relayed
    cmds.send(Cmd::Attack(b.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    pause(100).await;
    assert!(!texts(&mut a).iter().any(|t| t.contains("\"t\":\"swing\"")));
    // moving cancels
    cmds.send(Cmd::Pos(b.id, PosUpdate { x: 402.0, y: 0.0, z: -300.0, ..Default::default() })).await.unwrap();
    pause(100).await;
    assert!(texts(&mut a).iter().any(|t| t.contains("\"t\":\"logoutcancel\"")));
    pause(1100).await;
    assert!(!texts(&mut a).iter().any(|t| t.contains("\"t\":\"left\"")), "a cancelled logout does not log him out");
    // again, and wait it out
    cmds.send(Cmd::Chat(b.id, "/logout".into())).await.unwrap();
    pause(1400).await;
    let t = texts(&mut a);
    assert!(t.iter().any(|t| t.contains("\"t\":\"left\"")), "{t:?}");
    assert!(texts(&mut b).iter().any(|t| t.contains("\"t\":\"loggedout\"")));
}

#[tokio::test]
async fn someone_who_joins_during_a_logout_hears_it() {
    let cmds = start(300.0, 30.0).await;
    let b = join(&cmds, Some(1), (400.0, -300.0)).await;
    cmds.send(Cmd::Chat(b.id, "/logout".into())).await.unwrap();
    pause(100).await;
    let mut c = join(&cmds, None, (410.0, -300.0)).await;
    pause(100).await;
    assert!(texts(&mut c).iter().any(|t| t.contains("\"t\":\"logout\"")));
}
