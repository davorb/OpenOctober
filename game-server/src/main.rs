use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use game_server::proto::*;
use game_server::world::{self, Cmd, Config, JoinReq};

const USAGE: &str = "usage: game-server [--port 9000] [--radius 900] [--max-players 256] [--tick-hz 15] [--spawns ../game-viewer/data/mobspawns.json | --spawns none]";

#[tokio::main]
async fn main() {
    let (mut port, mut cfg) = (9000u16, Config { radius: 900.0, max_players: 256, tick_hz: 15, spawns: Some("../game-viewer/data/mobspawns.json".into()) });
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_default();
        match a.as_str() {
            "--port" => port = v.parse().expect(USAGE),
            "--radius" => cfg.radius = v.parse().expect(USAGE),
            "--max-players" => cfg.max_players = v.parse::<usize>().expect(USAGE).clamp(1, 60000),
            "--tick-hz" => cfg.tick_hz = v.parse::<u32>().expect(USAGE).clamp(1, 120),
            "--spawns" => cfg.spawns = (v != "none").then_some(v),
            _ => panic!("{USAGE}"),
        }
    }
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>(8192);
    tokio::spawn(world::run(cfg.clone(), cmd_rx));
    let listener = TcpListener::bind(("0.0.0.0", port)).await.expect("bind");
    println!("Darkfall game-server on ws://0.0.0.0:{port}  (radius {} m, {} Hz, max {} players)", cfg.radius, cfg.tick_hz, cfg.max_players);
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let _ = stream.set_nodelay(true);
        tokio::spawn(connection(stream, cmd_tx.clone()));
    }
}

async fn connection(stream: TcpStream, cmds: mpsc::Sender<Cmd>) {
    let mut wsc = WebSocketConfig::default();
    wsc.max_message_size = Some(4096);
    wsc.max_frame_size = Some(4096);
    let Ok(ws) = tokio_tungstenite::accept_async_with_config(stream, Some(wsc)).await else { return };
    let (mut sink, mut source) = ws.split();

    // the first frame must be a valid Join
    let first = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(m)) = source.next().await {
            if let Message::Text(t) = m {
                return serde_json::from_str::<ClientMsg>(&t).ok();
            }
        }
        None
    })
    .await;
    let Ok(Some(ClientMsg::Join { name, race, gender, gear })) = first else { return };

    let (tx, mut rx) = mpsc::channel::<Message>(128);
    let mut writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    });
    let (reply, reply_rx) = oneshot::channel();
    if cmds.send(Cmd::Join(JoinReq { name, race, gender, gear, tx: tx.clone(), reply })).await.is_err() {
        return;
    }
    let id = match reply_rx.await {
        Ok(Ok(id)) => id,
        Ok(Err(msg)) => {
            let _ = tx.send(Message::Text(serde_json::to_string(&ServerMsg::Error { msg }).unwrap())).await;
            drop(tx);
            let _ = writer.await;
            return;
        }
        Err(_) => return,
    };
    drop(tx); // the world holds the only sender: dropping the player closes the socket

    loop {
        tokio::select! {
            _ = &mut writer => break, // kicked, or the socket died
            m = source.next() => match m {
                Some(Ok(Message::Binary(b))) => {
                    if let Some(p) = PosUpdate::decode_frame(&b) {
                        if cmds.send(Cmd::Pos(id, p)).await.is_err() { break; }
                    }
                }
                Some(Ok(Message::Text(t))) => {
                    if let Ok(ClientMsg::Chat { text }) = serde_json::from_str(&t) {
                        if cmds.send(Cmd::Chat(id, text)).await.is_err() { break; }
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
        }
    }
    let _ = cmds.send(Cmd::Leave(id)).await;
}
