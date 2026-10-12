//! Load tester: `cargo run --release --bin bots -- [ws://127.0.0.1:9000] [count=100] [centre-x=-7712] [centre-z=-5600] [spread=400]`
//! Each bot joins with a random model, wanders between waypoints at run speed and chats now and then.
//! Coordinates are viewer (three.js) space; the default centre is the world start (game z 5600 -> -5600).
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use game_server::proto::*;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Default)]
struct Net {
    connected: AtomicU64,
    bytes_in: AtomicU64,
    snapshots: AtomicU64,
    entities: AtomicU64,
    chats: AtomicU64,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

const GEAR: [&str; 8] = ["", "Cloth", "Leather", "Chain", "Plate", "Banded", "Bone", "Dragon"];
const LINES: [&str; 6] = ["anyone near the keep?", "o/", "nice view up here", "heading north", "gg", "lag check"];

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: &str| a.get(i).cloned().unwrap_or_else(|| d.to_string());
    let url = arg(0, "ws://127.0.0.1:9000");
    let count: usize = arg(1, "100").parse().unwrap();
    let (cx, cz, spread): (f32, f32, f32) = (arg(2, "-7712").parse().unwrap(), arg(3, "-5600").parse().unwrap(), arg(4, "400").parse().unwrap());
    let net = Arc::new(Net::default());
    for i in 0..count {
        tokio::spawn(bot(i, url.clone(), (cx, cz, spread), net.clone()));
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let mut last = (0, 0, 0);
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (b, s, e) = (net.bytes_in.load(Relaxed), net.snapshots.load(Relaxed), net.entities.load(Relaxed));
        println!(
            "bots connected {}/{} | in {:.1} KB/s total ({:.1} KB/s per bot) | {:.1} snapshots/s/bot, {:.1} entities/snapshot | chats seen {}",
            net.connected.load(Relaxed), count, (b - last.0) as f64 / 5.0 / 1024.0, (b - last.0) as f64 / 5.0 / 1024.0 / count as f64,
            (s - last.1) as f64 / 5.0 / count as f64, (e - last.2) as f64 / (s - last.1).max(1) as f64, net.chats.load(Relaxed)
        );
        last = (b, s, e);
    }
}

async fn bot(i: usize, url: String, (cx, cz, spread): (f32, f32, f32), net: Arc<Net>) {
    let mut rng = Rng(0x9E3779B97F4A7C15 ^ ((i as u64 + 1) * 0xD1B54A32D192ED03));
    let Ok((ws, _)) = connect_async(url.as_str()).await else { eprintln!("bot {i}: connect failed"); return };
    let (mut tx, mut rx) = ws.split();
    let join = serde_json::json!({ "t": "join", "name": format!("Bot{i}"), "race": RACES[rng.next() as usize % 6], "gender": GENDERS[rng.next() as usize % 2], "gear": GEAR[rng.next() as usize % GEAR.len()] });
    if tx.send(Message::Text(join.to_string())).await.is_err() { return }
    net.connected.fetch_add(1, Relaxed);

    let spot = |r: &mut Rng| (cx + (r.f() * 2.0 - 1.0) * spread, cz + (r.f() * 2.0 - 1.0) * spread);
    let (mut x, mut z) = spot(&mut rng);
    let mut target = spot(&mut rng);
    let mut yaw = 0.0f32;
    let speed = 5.55f32; // run: 3.7 m/s * PLAYER_SCALE
    let dt = 1.0 / 15.0;
    let mut tick = tokio::time::interval(Duration::from_millis(66));
    let mut next_chat = 5.0 + rng.f() * 40.0;
    let mut t = 0.0f32;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                t += dt;
                let (dx, dz) = (target.0 - x, target.1 - z);
                let d = dx.hypot(dz);
                if d < 3.0 { target = spot(&mut rng); } else { x += dx / d * speed * dt; z += dz / d * speed * dt; yaw = (-dx).atan2(-dz); }
                let p = PosUpdate { x, y: 0.0, z, yaw, speed: (speed * 10.0) as u8, flags: FLAG_SNAP };
                if tx.send(Message::Binary(p.encode())).await.is_err() { break }
                if t > next_chat {
                    next_chat = t + 20.0 + rng.f() * 40.0;
                    let _ = tx.send(Message::Text(serde_json::json!({ "t": "chat", "text": LINES[rng.next() as usize % LINES.len()] }).to_string())).await;
                }
            }
            m = rx.next() => match m {
                Some(Ok(Message::Binary(b))) => {
                    net.bytes_in.fetch_add(b.len() as u64, Relaxed);
                    if b.len() >= 7 && b[0] == OP_SNAPSHOT { net.snapshots.fetch_add(1, Relaxed); net.entities.fetch_add(u16::from_le_bytes([b[5], b[6]]) as u64, Relaxed); }
                }
                Some(Ok(Message::Text(t))) => { if t.contains("\"chat\"") { net.chats.fetch_add(1, Relaxed); } }
                Some(Ok(_)) => {}
                _ => break,
            },
        }
    }
    net.connected.fetch_sub(1, Relaxed);
}
