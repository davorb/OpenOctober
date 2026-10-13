//! Living NPCs: level of detail by distance, no jumps when one gets promoted, legs finishing inside one long step, and NPCs reaching players in snapshots.
use std::time::Duration;

use game_server::npcs::*;
use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

fn places() -> Vec<Place> {
    // a row of towns 2 km apart along x
    (0..6).map(|i| Place { name: format!("T{i}"), x: i as f32 * 2000.0, z: 0.0, town: true }).collect()
}

fn dist(a: &PosUpdate, b: (f32, f32)) -> f32 {
    ((a.x - b.0).powi(2) + (a.z - b.1).powi(2)).sqrt()
}

#[test]
fn population_is_reproducible_and_labeled() {
    let a = Npcs::build(places(), &[(0, (0.0, 0.0))], 40, 7);
    let b = Npcs::build(places(), &[(0, (0.0, 0.0))], 40, 7);
    assert_eq!(a.npcs.len(), 40);
    for (x, y) in a.npcs.iter().zip(&b.npcs) {
        assert_eq!((x.info.name.as_str(), x.pos.x, x.pos.z), (y.info.name.as_str(), y.pos.x, y.pos.z));
        assert!(x.info.npc && x.pos.flags & FLAG_NPC != 0 && x.pos.flags & FLAG_SNAP != 0);
        assert!(x.id >= NPC_ID_BASE && x.info.id == x.id);
    }
    assert!(Npcs::build(Vec::new(), &[], 10, 1).npcs.is_empty(), "no places, no npcs");
}

#[test]
fn lod_follows_distance_to_the_nearest_player() {
    let mut w = Npcs::build(places(), &[(0, (0.0, 0.0))], 1, 3);
    let (nx, nz) = (w.npcs[0].pos.x, w.npcs[0].pos.z);
    for (off, want) in [(500.0, Lod::Active), (2000.0, Lod::Near), (8000.0, Lod::Far)] {
        // a far npc re-checks every 15 ticks: 20 ticks are always enough
        let mut c = LodCounts::default();
        for _ in 0..20 {
            c = w.step(1.0 / 15.0, &[(nx + off, nz)], 900.0);
        }
        assert_eq!(w.npcs[0].lod, want, "player {off} m away");
        assert_eq!((c.active, c.near, c.far), ((want == Lod::Active) as u32, (want == Lod::Near) as u32, (want == Lod::Far) as u32));
    }
}

#[test]
fn active_npcs_move_every_tick_and_far_ones_in_bursts() {
    let mut w = Npcs::build(places(), &[(0, (0.0, 0.0))], 30, 11);
    let start: Vec<(f32, f32)> = w.npcs.iter().map(|n| (n.pos.x, n.pos.z)).collect();
    let dt = 1.0 / 15.0;
    let mut longest_hop = 0.0f32;
    let mut prev = start.clone();
    for _ in 0..(15 * 600) {
        w.step(dt, &[(-50_000.0, 0.0)], 900.0); // nobody near: everything is far
        for (i, n) in w.npcs.iter().enumerate() {
            longest_hop = longest_hop.max(dist(&n.pos, prev[i]));
            prev[i] = (n.pos.x, n.pos.z);
        }
    }
    // ten minutes of life: some have travelled, none ever jumped further than one far step can carry them (plus the odd leg change)
    assert!(w.npcs.iter().zip(&start).any(|(n, s)| dist(&n.pos, *s) > 100.0), "somebody should have moved");
    assert!(longest_hop <= FAR_EVERY * 4.3 + 2.0, "far npcs hop {longest_hop} m in one step");
    assert!(w.npcs.iter().all(|n| n.pos.x.is_finite() && n.pos.z.is_finite()));
}

#[test]
fn promotion_catches_up_without_a_jump() {
    let mut w = Npcs::build(places(), &[(0, (0.0, 0.0))], 12, 5);
    let dt = 1.0 / 15.0;
    for _ in 0..(15 * 120) {
        w.step(dt, &[(-50_000.0, 0.0)], 900.0);
    }
    // a player appears next to every npc in turn: the npc lands where it would be "now", at most one far step away from where it was last seen
    for i in 0..w.npcs.len() {
        let before = (w.npcs[i].pos.x, w.npcs[i].pos.z);
        for _ in 0..16 {
            w.step(dt, &[(before.0 + 100.0, before.1)], 900.0);
            if w.npcs[i].lod == Lod::Active {
                break;
            }
        }
        assert_eq!(w.npcs[i].lod, Lod::Active);
        let hop = dist(&w.npcs[i].pos, before);
        assert!(hop <= 16.0 * dt * 4.3 + FAR_EVERY * 4.3 + 2.0, "npc {i} jumped {hop} m on promotion");
    }
}

#[test]
fn one_long_step_finishes_whole_legs() {
    let mut w = Npcs::build(places(), &[(0, (0.0, 0.0))], 8, 9);
    // 20 simulated minutes in 5 s steps: npcs have been through travel, rest and wander, and always stand on or between places
    for _ in 0..(12 * 20) {
        w.step(5.0, &[(-50_000.0, 0.0)], 900.0);
    }
    for n in &w.npcs {
        assert!(n.fatigue >= 0.0 && n.fatigue <= 1.0);
        assert!(n.pos.x > -200.0 && n.pos.x < 10_200.0 && n.pos.z.abs() < 200.0, "npc left the world: {:?}", n.pos);
    }
    assert!(w.npcs.iter().any(|n| n.at != n.home), "somebody should have changed town");
}

async fn join(cmds: &mpsc::Sender<Cmd>, at: (f32, f32)) -> mpsc::Receiver<Message> {
    let (tx, rx) = mpsc::channel(512);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { name: "t".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: at.0, y: 0.0, z: at.1, ..Default::default() })).await.unwrap();
    rx
}

#[tokio::test]
async fn npcs_show_up_in_snapshots_of_nearby_players_only() {
    let dir = std::env::temp_dir();
    let (pois, caps) = (dir.join("df_npc_pois.json"), dir.join("df_npc_caps.json"));
    // game z = 200 -> wire z = -200
    std::fs::write(&pois, r#"[{"name":"Town","type":"Village","x":100,"z":200}]"#).unwrap();
    std::fs::write(&caps, r#"[{"name":"Shoal","race":"Alfar","x":100,"y":0,"z":200}]"#).unwrap();
    let cfg = Config { radius: 900.0, max_players: 8, tick_hz: 15, spawns: None, weapons: None, pvp: true, npcs: 5, pois: Some(pois.to_string_lossy().into()), capitals: Some(caps.to_string_lossy().into()), npc_seed: 1, db: None, nav: None };
    let (cmds, rx) = mpsc::channel(64);
    tokio::spawn(world::run(cfg, rx));

    let mut near = join(&cmds, (120.0, -190.0)).await;
    let mut far = join(&cmds, (9000.0, 9000.0)).await;
    tokio::time::sleep(Duration::from_millis(1800)).await;

    let (mut welcome_npcs, mut seen) = (0, std::collections::HashSet::new());
    while let Ok(m) = near.try_recv() {
        match m {
            Message::Text(t) if t.contains("\"welcome\"") => welcome_npcs = t.matches("\"npc\":true").count(),
            Message::Binary(b) if b[0] == OP_SNAPSHOT => {
                let n = u16::from_le_bytes([b[5], b[6]]) as usize;
                for i in 0..n {
                    let o = 7 + i * 20;
                    let id = u16::from_le_bytes([b[o], b[o + 1]]);
                    if id >= NPC_ID_BASE {
                        assert!(b[o + 19] & FLAG_NPC != 0 && b[o + 19] & FLAG_SNAP != 0, "npc flags on the wire");
                        seen.insert(id);
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(welcome_npcs, 5, "all npcs are in the welcome roster");
    assert_eq!(seen.len(), 5, "the five npcs of the village are visible: {seen:?}");

    while let Ok(m) = far.try_recv() {
        if let Message::Binary(b) = m {
            if b[0] == OP_SNAPSHOT {
                assert_eq!(u16::from_le_bytes([b[5], b[6]]), 0, "a far player sees nobody");
            }
        }
    }
}
