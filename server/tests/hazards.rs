//! Harmful liquids: the hazard part of the baked nav files, and players hurt by standing or swimming in lava.
use std::time::Duration;

use server::nav::{Hazard, Nav, NavTile};
use server::proto::*;
use server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const SURFACE: f32 = 176.0;

/// A baked tile at (0, 0) whose cell (50, 50), x and z from 100 to 102, is lava.
fn baked(version: u32) -> Vec<u8> {
    let mut b = b"DNAV".to_vec();
    b.extend(version.to_le_bytes());
    b.extend([0; 8]);
    b.extend(256u16.to_le_bytes());
    b.extend(512u16.to_le_bytes());
    b.extend(vec![0; 257 * 257 * 2 + 512 * 512]);
    b.extend(1u32.to_le_bytes());
    for v in [50, 50, ((SURFACE + 500.0) * 20.0) as u16, 1] {
        b.extend(v.to_le_bytes());
    }
    b
}

#[test]
fn a_hazard_reaches_from_its_surface_to_a_swimmers_feet() {
    let nav = Nav::from_tiles(vec![NavTile::parse(&baked(2)).unwrap()]);
    assert_eq!(nav.hazard(101.0, SURFACE - 1.8, 101.0), Some(Hazard::Lava), "swimming");
    assert_eq!(nav.hazard(101.0, SURFACE - 0.5, 101.0), Some(Hazard::Lava), "wading");
    assert_eq!(nav.hazard(101.0, SURFACE + 1.0, 101.0), None, "on a bridge over it");
    assert_eq!(nav.hazard(101.0, SURFACE - 30.0, 101.0), None, "in a cave far below");
    assert_eq!(nav.hazard(104.0, SURFACE - 1.0, 101.0), None, "beside it");
}

#[test]
fn old_and_truncated_bakes_are_refused() {
    assert!(NavTile::parse(&baked(1)).is_err());
    let b = baked(2);
    assert!(NavTile::parse(&b[..b.len() - 1]).is_err());
}

async fn join(cmds: &mpsc::Sender<Cmd>, y: f32, flags: u8) -> mpsc::Receiver<Message> {
    let (tx, rx) = mpsc::channel(4096);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { account: None, name: "t".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: 101.0, y, z: 101.0, flags, ..Default::default() })).await.unwrap();
    rx
}

#[tokio::test]
async fn lava_hurts_whoever_is_in_it_and_kills() {
    let dir = std::env::temp_dir().join(format!("df_hazards_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.nav"), baked(2)).unwrap();
    let cfg = Config { max_speed: 0.0, max_fly_speed: 0.0, ghost_secs: 0.0, logout_secs: 30.0, radius: 900.0, max_players: 8, tick_hz: 30, spawns: None, weapons: None, pvp: true, npcs: 0, pois: None, capitals: None, npc_seed: 1, db: None, nav: Some(dir.to_string_lossy().into()), npc_time_scale: 1.0, own_weapons: false, body_secs: 45.0, tomb_secs: 3600, loot_secs: 3.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    let mut swimmer = join(&cmds, SURFACE - 1.8, FLAG_SWIM).await; // id 0
    let _above = join(&cmds, SURFACE + 2.0, 0).await; // id 1
    let _flyer = join(&cmds, SURFACE - 1.8, FLAG_FLY).await; // id 2
    tokio::time::sleep(Duration::from_millis(5500)).await;
    let (mut hit, mut died) = (Vec::new(), Vec::new());
    while let Ok(m) = swimmer.try_recv() {
        let Message::Text(t) = m else { continue };
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        match v["t"].as_str() {
            Some("hit") => hit.push((v["i"].as_u64().unwrap(), v["grp"].to_string())),
            Some("died") => died.push(v["i"].as_u64().unwrap()),
            _ => {}
        }
    }
    assert_eq!(hit, vec![(0, "\"lava\"".to_string()); 4], "120 hit points at 30 a second, and only the swimmer");
    assert_eq!(died, vec![0]);
}
