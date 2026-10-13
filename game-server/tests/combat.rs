//! Combat end to end through the world task: a swing hits and kills a mob, a mob fights back, players hurt each other, a blocker takes less.
use std::time::Duration;

use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const WEAPONS: &str = r#"{"weapons":{"r_Test Sword":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":30},
    "r_Test Bow":{"group":"bow","kind":"ranged","rank":30},
    "r_Test Doom":{"group":"sword","kind":"melee","reach":2.1,"angle":60,"rank":400}},"clipDur":{"singlehand_attack_01":1.0,"bow_fire":0.83}}"#;

struct Client {
    id: u16,
    rx: mpsc::Receiver<Message>,
}

async fn start(spawns: Option<&str>, tag: &str) -> mpsc::Sender<Cmd> {
    let dir = std::env::temp_dir();
    let (w, s) = (dir.join(format!("df_combat_w_{tag}.json")), dir.join(format!("df_combat_s_{tag}.json")));
    std::fs::write(&w, WEAPONS).unwrap();
    if let Some(sp) = spawns {
        std::fs::write(&s, sp).unwrap();
    }
    let cfg = Config { max_speed: 0.0, max_fly_speed: 0.0, ghost_secs: 300.0, logout_secs: 30.0, radius: 900.0, max_players: 8, tick_hz: 30, spawns: spawns.map(|_| s.to_string_lossy().into()), weapons: Some(w.to_string_lossy().into()), pvp: true, npcs: 0, pois: None, capitals: None, npc_seed: 1, db: None, nav: None, npc_time_scale: 1.0, own_weapons: false, body_secs: 45.0, tomb_secs: 3600, loot_secs: 3.0, items: None, vendors: None };
    let (cmds, rx) = mpsc::channel(256);
    tokio::spawn(world::run(cfg, rx));
    cmds
}

async fn join(cmds: &mpsc::Sender<Cmd>, at: (f32, f32), yaw: f32) -> Client {
    let (tx, rx) = mpsc::channel(4096);
    let (reply, reply_rx) = oneshot::channel();
    cmds.send(Cmd::Join(JoinReq { account: None, name: "t".into(), race: "hum".into(), gender: "m".into(), gear: String::new(), tx, reply })).await.unwrap();
    let id = reply_rx.await.unwrap().unwrap();
    cmds.send(Cmd::Pos(id, PosUpdate { x: at.0, y: 0.0, z: at.1, yaw, ..Default::default() })).await.unwrap();
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

/// Position of the first mob in the latest OP_MOBS frame.
fn first_mob(c: &mut Client) -> Option<(f32, f32)> {
    let mut last = None;
    while let Ok(m) = c.rx.try_recv() {
        if let Message::Binary(b) = m {
            if b[0] == OP_MOBS && u16::from_le_bytes([b[5], b[6]]) > 0 {
                let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
                last = Some((f(11), f(19)));
            }
        }
    }
    last
}

#[tokio::test]
async fn swing_hits_and_kills_a_mob() {
    let cmds = start(Some(r#"[{"id":1,"x":100,"z":200,"mobs":[{"id":7,"name":"Rat","qty":1,"difficulty":"Very Easy"}]}]"#), "kill").await;
    let mut me = join(&cmds, (100.0, -200.0), 0.0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (mx, mz) = first_mob(&mut me).expect("mob frame");
    // stand 2 m south of the mob, looking north (yaw 0 looks along -z)
    cmds.send(Cmd::Pos(me.id, PosUpdate { x: mx, y: 0.0, z: mz + 2.0, yaw: 0.0, ..Default::default() })).await.unwrap();
    cmds.send(Cmd::Equip(me.id, "r_Test Sword".into(), true)).await.unwrap();
    texts(&mut me);
    let (mut hits, mut died) = (0, false);
    for _ in 0..5 {
        cmds.send(Cmd::Attack(me.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        for t in texts(&mut me) {
            if t.contains("\"t\":\"hit\"") && t.contains("\"v\":\"m\"") {
                hits += 1;
                assert!(t.contains("\"grp\":\"sword\""), "{t}");
            }
            died |= t.contains("\"t\":\"died\"") && t.contains("\"v\":\"m\"");
        }
        if died {
            break;
        }
    }
    assert!(hits >= 1, "the swing connected");
    assert!(died, "a 25 hp rat dies to a few sword blows");
}

#[tokio::test]
async fn a_swing_in_the_wrong_direction_misses() {
    let cmds = start(Some(r#"[{"id":1,"x":100,"z":200,"mobs":[{"id":7,"name":"Rat","qty":1,"difficulty":"Very Easy"}]}]"#), "miss").await;
    let mut me = join(&cmds, (100.0, -200.0), 0.0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (mx, mz) = first_mob(&mut me).expect("mob frame");
    // the mob is south of us, but we look north
    cmds.send(Cmd::Pos(me.id, PosUpdate { x: mx, y: 0.0, z: mz - 2.0, yaw: 0.0, ..Default::default() })).await.unwrap();
    cmds.send(Cmd::Equip(me.id, "r_Test Sword".into(), true)).await.unwrap();
    cmds.send(Cmd::Attack(me.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(!texts(&mut me).iter().any(|t| t.contains("\"t\":\"hit\"")));
}

#[tokio::test]
async fn an_aggressive_mob_fights_back() {
    let cmds = start(Some(r#"[{"id":1,"x":100,"z":200,"mobs":[{"id":7,"name":"Wolf","qty":1,"difficulty":"Easy"}]}]"#), "aggro").await;
    let mut me = join(&cmds, (100.0, -200.0), 0.0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (mx, mz) = first_mob(&mut me).expect("mob frame");
    cmds.send(Cmd::Pos(me.id, PosUpdate { x: mx, y: 0.0, z: mz + 12.0, yaw: 0.0, ..Default::default() })).await.unwrap();
    texts(&mut me);
    let mut hurt = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        hurt |= texts(&mut me).iter().any(|t| t.contains("\"t\":\"hit\"") && t.contains("\"v\":\"p\"") && t.contains("\"am\":true"));
        if hurt {
            break;
        }
    }
    assert!(hurt, "an Easy mob chases and bites a player 12 m away");
}

#[tokio::test]
async fn players_hurt_each_other_and_a_block_helps() {
    let cmds = start(None, "pvp").await;
    let mut a = join(&cmds, (500.0, -500.0), 0.0).await;
    let mut b = join(&cmds, (500.0, -502.5), std::f32::consts::PI).await; // 2.5 m north of a, looking back at him
    for c in [&a.id, &b.id] {
        cmds.send(Cmd::Equip(*c, "r_Test Sword".into(), true)).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    texts(&mut a);
    texts(&mut b);
    cmds.send(Cmd::Attack(a.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    let plain = texts(&mut b).into_iter().find(|t| t.contains("\"t\":\"hit\"") && t.contains(&format!("\"i\":{}", b.id))).expect("b was hit");
    let dmg = |t: &str| serde_json::from_str::<serde_json::Value>(t).unwrap()["dmg"].as_f64().unwrap();
    let plain_dmg = dmg(&plain);
    assert!(plain_dmg > 15.0);

    // b guards and is hit again
    cmds.send(Cmd::Block(b.id, true)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await; // a's cooldown
    texts(&mut b);
    cmds.send(Cmd::Attack(a.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    let guarded = texts(&mut b).into_iter().find(|t| t.contains("\"t\":\"hit\"")).expect("b was hit again");
    assert!(guarded.contains("\"blocked\":true"), "{guarded}");
    assert!(dmg(&guarded) < plain_dmg * 0.4, "{} vs {}", dmg(&guarded), plain_dmg);
}

#[tokio::test]
async fn equip_is_relayed_and_unknown_weapons_refused() {
    let cmds = start(None, "equip").await;
    let mut a = join(&cmds, (0.0, 0.0), 0.0).await;
    let mut b = join(&cmds, (5.0, 0.0), 0.0).await;
    texts(&mut a);
    texts(&mut b);
    cmds.send(Cmd::Equip(a.id, "r_No Such Thing".into(), true)).await.unwrap();
    cmds.send(Cmd::Equip(a.id, "r_Test Bow".into(), true)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let seen = texts(&mut b);
    assert!(seen.iter().any(|t| t.contains("\"t\":\"equip\"") && t.contains("r_Test Bow") && t.contains("\"drawn\":true")), "{seen:?}");
    assert!(!seen.iter().any(|t| t.contains("No Such Thing")));
}

#[tokio::test]
async fn an_arrow_flies_downrange_and_hits_a_mob() {
    let cmds = start(Some(r#"[{"id":1,"x":100,"z":200,"mobs":[{"id":7,"name":"Rat","qty":1,"difficulty":"Very Easy"}]}]"#), "arrow").await;
    let mut me = join(&cmds, (100.0, -200.0), 0.0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (mx, mz) = first_mob(&mut me).expect("mob frame");
    cmds.send(Cmd::Pos(me.id, PosUpdate { x: mx, y: 0.0, z: mz + 30.0, yaw: 0.0, ..Default::default() })).await.unwrap();
    cmds.send(Cmd::Equip(me.id, "r_Test Bow".into(), true)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    texts(&mut me);
    // from the shooter's hand towards the mob (along -z)
    cmds.send(Cmd::Shoot(me.id, [mx, 1.6, mz + 29.0], [0.0, 0.0, -1.0], 1.0, false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    let t = texts(&mut me).into_iter().find(|t| t.contains("\"t\":\"hit\"")).expect("the arrow hit");
    assert!(t.contains("\"grp\":\"bow\"") && t.contains("\"v\":\"m\""), "{t}");
    // shooting without a drawn bow does nothing
    cmds.send(Cmd::Equip(me.id, "r_Test Sword".into(), true)).await.unwrap();
    texts(&mut me);
    cmds.send(Cmd::Shoot(me.id, [mx, 1.6, mz + 29.0], [0.0, 0.0, -1.0], 1.0, false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!texts(&mut me).iter().any(|t| t.contains("\"t\":\"hit\"")));
}

#[tokio::test]
async fn jumping_players_are_hit_but_flying_ones_are_not() {
    let cmds = start(None, "jump").await;
    let mut a = join(&cmds, (700.0, -700.0), 0.0).await;
    let mut b = join(&cmds, (700.0, -702.5), std::f32::consts::PI).await;
    for c in [a.id, b.id] {
        cmds.send(Cmd::Equip(c, "r_Test Sword".into(), true)).await.unwrap();
    }
    let at = |id: u16, flags: u8| PosUpdate { x: 700.0, y: 3.0, z: if id == 0 { -700.0 } else { -702.5 }, yaw: if id == 0 { 0.0 } else { std::f32::consts::PI }, speed: 0, flags };
    // b jumps (FLAG_AIR): still hit
    cmds.send(Cmd::Pos(b.id, at(b.id, FLAG_AIR))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    texts(&mut b);
    cmds.send(Cmd::Attack(a.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(texts(&mut b).iter().any(|t| t.contains("\"t\":\"hit\"")), "a jumping player is hittable");
    // b flies (FLAG_FLY): out of the fight
    cmds.send(Cmd::Pos(b.id, at(b.id, FLAG_AIR | FLAG_FLY))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    texts(&mut b);
    cmds.send(Cmd::Attack(a.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(!texts(&mut b).iter().any(|t| t.contains("\"t\":\"hit\"")), "a flying camera is not");
    let _ = &mut a;
}

#[tokio::test]
async fn bots_without_height_are_hit_by_arrows_at_real_terrain_height() {
    let cmds = start(None, "snap").await;
    let mut a = join(&cmds, (900.0, -900.0), 0.0).await;
    let mut bot = join(&cmds, (900.0, -930.0), 0.0).await;
    // a bot reports no height (y = 0, FLAG_SNAP); the shooter stands on terrain at y = 520
    cmds.send(Cmd::Pos(bot.id, PosUpdate { x: 900.0, y: 0.0, z: -930.0, flags: FLAG_SNAP, ..Default::default() })).await.unwrap();
    cmds.send(Cmd::Pos(a.id, PosUpdate { x: 900.0, y: 520.0, z: -900.0, ..Default::default() })).await.unwrap();
    cmds.send(Cmd::Equip(a.id, "r_Test Bow".into(), true)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    texts(&mut a);
    cmds.send(Cmd::Shoot(a.id, [900.0, 521.6, -901.0], [0.0, 0.0, -1.0], 1.0, false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    let hit = texts(&mut a).into_iter().find(|t| t.contains("\"t\":\"hit\"")).expect("the arrow hit the bot");
    assert!(hit.contains("\"snap\":true") && hit.contains(&format!("\"i\":{}", bot.id)), "{hit}");
    let _ = &mut bot;
}

#[tokio::test]
async fn a_dead_player_leaves_a_corpse_that_stays_put_and_late_joiners_see() {
    let cmds = start(None, "corpse").await;
    let mut a = join(&cmds, (300.0, -300.0), 0.0).await;
    let mut b = join(&cmds, (300.0, -302.5), std::f32::consts::PI).await;
    cmds.send(Cmd::Equip(a.id, "r_Test Doom".into(), true)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    texts(&mut a);
    cmds.send(Cmd::Attack(a.id, "slash".into(), "singlehand_attack_01".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    let seen = texts(&mut a);
    let corpse = seen.iter().find(|t| t.contains("\"t\":\"corpse\"")).expect("a corpse message");
    let v: serde_json::Value = serde_json::from_str(corpse).unwrap();
    assert!((v["x"].as_f64().unwrap() - 300.0).abs() < 0.01 && (v["z"].as_f64().unwrap() + 302.5).abs() < 0.01);
    // the dead player "walks away" (a bot keeps sending positions): the body does not move
    cmds.send(Cmd::Pos(b.id, PosUpdate { x: 350.0, y: 0.0, z: -350.0, ..Default::default() })).await.unwrap();
    // a player who joins later is told about the body
    let mut c = join(&cmds, (310.0, -310.0), 0.0).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let late = texts(&mut c);
    assert!(late.iter().any(|t| t.contains("\"t\":\"corpse\"") && t.contains(&format!("\"cid\":{}", v["cid"]))), "{late:?}");
    let _ = &mut b;
}
