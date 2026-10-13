//! Abuse limits: the movement limit and shot validation in the world task, and (through the real binary) the origin check, per-IP limits and frame flood kick.
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const WEAPONS: &str = r#"{"weapons":{"r_Test Bow":{"group":"bow","kind":"ranged","rank":30}},"clipDur":{"bow_fire":0.83}}"#;

struct Client {
    id: u16,
    rx: mpsc::Receiver<Message>,
}

async fn start(max_speed: f32, max_fly_speed: f32, tag: &str) -> mpsc::Sender<Cmd> {
    let w = std::env::temp_dir().join(format!("df_abuse_w_{tag}.json"));
    std::fs::write(&w, WEAPONS).unwrap();
    let cfg = Config { max_speed, max_fly_speed, radius: 900.0, max_players: 8, tick_hz: 30, spawns: None, weapons: Some(w.to_string_lossy().into()), pvp: true, npcs: 0, pois: None, capitals: None, npc_seed: 1, db: None, nav: None, npc_time_scale: 1.0, own_weapons: false, body_secs: 45.0, tomb_secs: 3600, loot_secs: 3.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    cmds
}

async fn join(cmds: &mpsc::Sender<Cmd>) -> Client {
    let (tx, rx) = mpsc::channel(4096);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "t".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    Client { id: reply_rx.await.unwrap().unwrap(), rx }
}

fn pos(x: f32, z: f32, flags: u8) -> PosUpdate {
    PosUpdate { x, y: 0.0, z, yaw: 0.0, speed: 0, flags }
}

/// Where the latest snapshot `c` got puts player `who`.
fn seen_at(c: &mut Client, who: u16) -> Option<(f32, f32)> {
    let mut last = None;
    while let Ok(m) = c.rx.try_recv() {
        if let Message::Binary(b) = m {
            if b[0] == OP_SNAPSHOT {
                let n = u16::from_le_bytes([b[5], b[6]]) as usize;
                for i in 0..n {
                    let o = 7 + i * 20;
                    if u16::from_le_bytes([b[o], b[o + 1]]) == who {
                        let f = |j: usize| f32::from_le_bytes([b[o + j], b[o + j + 1], b[o + j + 2], b[o + j + 3]]);
                        last = Some((f(2), f(10)));
                    }
                }
            }
        }
    }
    last
}

#[tokio::test]
async fn a_jump_is_cut_to_what_the_player_could_have_walked() {
    let cmds = start(10.0, 100.0, "jump").await;
    let (mut watcher, mover) = (join(&cmds).await, join(&cmds).await);
    cmds.send(Cmd::Pos(watcher.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Pos(mover.id, pos(0.0, 0.0, 0))).await.unwrap(); // the first update puts him anywhere
    cmds.send(Cmd::Pos(mover.id, pos(500.0, 0.0, 0))).await.unwrap(); // a teleport
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (x, _) = seen_at(&mut watcher, mover.id).expect("snapshot");
    assert!(x > 0.0 && x < 30.0, "cut to the walking allowance (burst 20 m + a moment), got {x}");
    // walking at 5 m/s is not touched
    for i in 1..=10 {
        cmds.send(Cmd::Pos(mover.id, pos(x + i as f32 * 0.5, 0.0, 0))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (x2, _) = seen_at(&mut watcher, mover.id).unwrap();
    assert!((x2 - (x + 5.0)).abs() < 0.01, "walked 5 m: {x} -> {x2}");
}

#[tokio::test]
async fn flooding_updates_does_not_buy_extra_speed() {
    let cmds = start(10.0, 100.0, "flood").await;
    let (mut watcher, mover) = (join(&cmds).await, join(&cmds).await);
    cmds.send(Cmd::Pos(watcher.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Pos(mover.id, pos(0.0, 0.0, 0))).await.unwrap();
    for i in 1..=200 {
        cmds.send(Cmd::Pos(mover.id, pos(i as f32 * 5.0, 0.0, 0))).await.unwrap(); // asks for 1000 m in no time
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (x, _) = seen_at(&mut watcher, mover.id).unwrap();
    assert!(x < 30.0, "got {x}");
}

#[tokio::test]
async fn flying_has_its_own_limit_and_zero_means_none() {
    let cmds = start(10.0, 100.0, "fly").await;
    let (mut watcher, flyer) = (join(&cmds).await, join(&cmds).await);
    cmds.send(Cmd::Pos(watcher.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Pos(flyer.id, pos(0.0, 0.0, FLAG_FLY))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1000)).await; // a second of flying allowance: about 100 m
    cmds.send(Cmd::Pos(flyer.id, pos(1500.0, 0.0, FLAG_FLY))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (x, _) = seen_at(&mut watcher, flyer.id).unwrap();
    assert!(x > 90.0 && x < 140.0, "flying is held to the fly limit (100 m/s), not the walking one and not none; got {x}");

    let cmds = start(0.0, 0.0, "off").await;
    let (mut watcher, other) = (join(&cmds).await, join(&cmds).await);
    cmds.send(Cmd::Pos(watcher.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Pos(other.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Pos(other.id, pos(800.0, 0.0, 0))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(seen_at(&mut watcher, other.id).unwrap().0, 800.0);
}

#[tokio::test]
async fn a_trusted_player_and_a_respawn_may_jump() {
    let cmds = start(10.0, 100.0, "trust").await;
    let (mut watcher, local) = (join(&cmds).await, join(&cmds).await);
    cmds.send(Cmd::Pos(watcher.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Trust(local.id)).await.unwrap();
    cmds.send(Cmd::Pos(local.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Pos(local.id, pos(600.0, 0.0, 0))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(seen_at(&mut watcher, local.id).unwrap().0, 600.0);
}

#[tokio::test]
async fn a_shot_with_a_poisonous_number_is_refused() {
    let cmds = start(0.0, 0.0, "shot").await;
    let (mut watcher, me) = (join(&cmds).await, join(&cmds).await);
    cmds.send(Cmd::Pos(watcher.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Pos(me.id, pos(0.0, 0.0, 0))).await.unwrap();
    cmds.send(Cmd::Equip(me.id, "r_Test Bow".into(), true)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    while watcher.rx.try_recv().is_ok() {}
    for (dir, power) in [([0.0, 0.0, -1.0], f32::INFINITY), ([1.0e30, 0.0, 0.0], 1.0), ([f32::INFINITY, 0.0, 0.0], 1.0), ([0.0, 0.0, -1.0], f32::NAN)] {
        cmds.send(Cmd::Shoot(me.id, [0.0, 1.0, 0.0], dir, power, false)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
    let mut shots = 0;
    while let Ok(m) = watcher.rx.try_recv() {
        if let Message::Text(t) = m {
            shots += t.contains("\"shoot\"") as usize;
        }
    }
    assert_eq!(shots, 0, "no poisoned shot is relayed");
    cmds.send(Cmd::Shoot(me.id, [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], 1.0, false)).await.unwrap(); // a good one still goes
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut ok = false;
    while let Ok(m) = watcher.rx.try_recv() {
        if let Message::Text(t) = m {
            ok |= t.contains("\"shoot\"");
        }
    }
    assert!(ok);
}

// ---- the real binary -------------------------------------------------------------------------------------------------------------------

struct Server(Child, u16);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn serve(extra: &[&str]) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut args: Vec<String> = ["--port", &port.to_string(), "--spawns", "none", "--weapons", "none", "--npcs", "0", "--db", "none", "--nav", "none", "--items", "none", "--vendors", "none", "--pois", "none", "--capitals", "none"].map(String::from).into();
    args.extend(extra.iter().map(|s| s.to_string()));
    let child = Command::new(env!("CARGO_BIN_EXE_game-server")).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Server(child, port)
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(port: u16, headers: &[(&str, &str)]) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://127.0.0.1:{port}").into_client_request().unwrap();
    for (k, v) in headers {
        req.headers_mut().insert(tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|(ws, _)| ws)
}

const JOIN: &str = r#"{"t":"join","name":"x","race":"hum","gender":"m"}"#;

async fn joined(ws: &mut Ws) -> bool {
    ws.send(Message::Text(JOIN.into())).await.unwrap();
    while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
        if let Message::Text(t) = m {
            return t.contains("welcome");
        }
    }
    false
}

#[tokio::test]
async fn pages_of_other_sites_are_refused() {
    let s = serve(&["--origins", "https://play.example.com"]).await;
    assert!(connect(s.1, &[("origin", "https://evil.example")]).await.is_err());
    assert!(connect(s.1, &[("origin", "null")]).await.is_err());
    assert!(connect(s.1, &[("origin", "https://play.example.com")]).await.is_ok());
    assert!(connect(s.1, &[("origin", "http://localhost:1337")]).await.is_ok());
    assert!(connect(s.1, &[("origin", "http://127.0.0.1:1337")]).await.is_ok());
    assert!(connect(s.1, &[]).await.is_ok(), "no Origin: not a browser");
    // a page served from the host the client reached us at is fine (host header is 127.0.0.1:port here)
    assert!(connect(s.1, &[("origin", "http://127.0.0.1:8080")]).await.is_ok());
    let s = serve(&["--origins", "any"]).await;
    assert!(connect(s.1, &[("origin", "https://evil.example")]).await.is_ok());
}

#[tokio::test]
async fn too_many_connections_from_one_address_are_refused_but_not_from_localhost() {
    let s = serve(&["--max-per-ip", "2", "--conns-per-min", "0"]).await;
    // local connections: no limit
    let mut local = Vec::new();
    for _ in 0..6 {
        local.push(connect(s.1, &[]).await.expect("localhost is exempt"));
    }
    drop(local);
    // behind a proxy on this machine the address comes from X-Forwarded-For
    let fwd = [("x-forwarded-for", "203.0.113.7")];
    let (a, b) = (connect(s.1, &fwd).await.unwrap(), connect(s.1, &fwd).await.unwrap());
    assert!(connect(s.1, &fwd).await.is_err(), "third from the same address");
    assert!(connect(s.1, &[("x-forwarded-for", "203.0.113.8")]).await.is_ok(), "another address is fine");
    drop(a);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(connect(s.1, &fwd).await.is_ok(), "a slot is free again");
    drop(b);
}

#[tokio::test]
async fn connection_attempts_per_minute_are_limited() {
    let s = serve(&["--max-per-ip", "0", "--conns-per-min", "3"]).await;
    let fwd = [("x-forwarded-for", "203.0.113.9")];
    for _ in 0..3 {
        connect(s.1, &fwd).await.unwrap();
    }
    assert!(connect(s.1, &fwd).await.is_err());
}

#[tokio::test]
async fn a_client_flooding_frames_is_kicked() {
    let s = serve(&[]).await;
    let mut ws = connect(s.1, &[]).await.unwrap();
    assert!(joined(&mut ws).await);
    let mut closed = false;
    for _ in 0..2000 {
        if ws.send(Message::Text(r#"{"t":"block","on":true}"#.into())).await.is_err() {
            closed = true;
            break;
        }
    }
    if !closed {
        // the kick may need a moment to show
        closed = loop {
            match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
                Ok(Some(Ok(_))) => continue,
                _ => break true,
            }
        };
    }
    assert!(closed);
    // an ordinary client at 15 Hz is left alone
    let mut ws = connect(s.1, &[]).await.unwrap();
    assert!(joined(&mut ws).await);
    for _ in 0..45 {
        ws.send(Message::Binary(PosUpdate::default().encode())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(66)).await;
    }
    ws.send(Message::Text(r#"{"t":"chat","text":"still here"}"#.into())).await.unwrap();
}
