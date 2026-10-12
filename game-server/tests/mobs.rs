//! A player standing next to a spawn point receives the mob roster and `OP_MOBS` frames; a far-away player receives none.
use std::time::Duration;

use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

async fn join(cmds: &mpsc::Sender<Cmd>, at: (f32, f32)) -> mpsc::Receiver<Message> {
    let (tx, rx) = mpsc::channel(256);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "t".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: at.0, y: 0.0, z: at.1, ..Default::default() })).await.unwrap();
    rx
}

#[tokio::test]
async fn mobs_reach_nearby_players_only() {
    let path = std::env::temp_dir().join("df_mobs_test.json");
    // game z = 200 -> wire z = -200
    std::fs::write(&path, r#"[{"id":1,"x":100,"z":200,"mobs":[{"id":7,"name":"Troll","qty":3,"difficulty":"Hard"}]}]"#).unwrap();
    let cfg = Config { radius: 900.0, max_players: 8, tick_hz: 15, spawns: Some(path.to_string_lossy().into()), weapons: None, pvp: true };
    let (cmds, rx) = mpsc::channel(64);
    tokio::spawn(world::run(cfg, rx));

    let mut near = join(&cmds, (120.0, -190.0)).await;
    let mut far = join(&cmds, (9000.0, 9000.0)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (mut roster, mut mob_frames) = (false, Vec::new());
    while let Ok(m) = near.try_recv() {
        match m {
            Message::Text(t) if t.contains("\"mobtypes\"") => roster = t.contains("Troll"),
            Message::Binary(b) if b[0] == OP_MOBS => mob_frames.push(b),
            _ => {}
        }
    }
    assert!(roster, "mob type roster sent after welcome");
    let f = mob_frames.last().expect("a mob frame");
    assert_eq!(u16::from_le_bytes([f[5], f[6]]), 3);
    assert_eq!(f.len(), 7 + 3 * 22);
    assert_eq!(u16::from_le_bytes([f[9], f[10]]), 7); // type of the first mob
    let z = f32::from_le_bytes([f[7 + 4 + 8], f[7 + 4 + 9], f[7 + 4 + 10], f[7 + 4 + 11]]);
    assert!((z + 200.0).abs() < 20.0, "wire z is negated game z, got {z}");

    while let Ok(m) = far.try_recv() {
        assert!(!matches!(m, Message::Binary(b) if b[0] == OP_MOBS), "far player must not get mobs");
    }
}
