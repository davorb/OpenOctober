use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use server::accounts::{self, Accounts, Profile, SessionGuard, SignUp};
use server::proto::*;
use server::world::{self, Cmd, Config, JoinReq};

const USAGE: &str = "usage: server [--port 1338] [--radius 900] [--max-players 256] [--tick-hz 15] [--spawns ../game/data/mobspawns.json | --spawns none] [--weapons ../game/data/weapons.json | --weapons none] [--pvp on|off] [--npcs 500] [--pois ../game/data/pois.json] [--capitals ../game/data/spawns.json] [--npc-seed 1] [--db npcs.db | --db none] [--nav data/nav | --nav none] [--npc-time-scale 1] [--own-weapons on|off] [--body-secs 7] [--tomb-secs 3600] [--loot-secs 3] [--items ../game/data/items.json] [--vendors ../game/data/vendors.json | --items none] [--max-per-ip 4] [--conns-per-min 30] [--origins https://play.example.com,... | --origins any] [--max-speed 20] [--max-fly-speed 400] [--ghost-secs 300] [--logout-secs 30] [--accounts accounts.db] [--make-admin USERNAME] [--set-password USERNAME:NEWPASSWORD]";

// Abuse limits (see `Net`). Connections from the local machine (127.0.0.1, ::1) skip the per-IP limits and the movement limit.
/// A client may send this many frames per second on average, with a burst of `MSG_BURST`; more are dropped, and a client that keeps on is kicked.
const MSG_RATE: f32 = 40.0;
const MSG_BURST: f32 = 80.0;
const MSG_MAX_OVER: u32 = 100;
/// A player who sends nothing the server understands for this long is kicked (a real client sends its position 15 times a second).
const IDLE_KICK: Duration = Duration::from_secs(15 * 60);
/// The websocket handshake has to be done within this long.
const HANDSHAKE_SECS: u64 = 10;
/// A connection that is logging in (or loading the world) and sends nothing for this long is dropped.
const PRE_ENTER_IDLE: Duration = Duration::from_secs(10 * 60);
/// Wrong passwords / refused sign-ups on one connection before it is dropped.
const MAX_AUTH_ERRORS: u32 = 10;
/// Seconds between saves of where a character is.
const POS_SAVE_SECS: u64 = 30;

struct Net {
    /// Browser pages allowed to open a connection (`Origin` header), besides pages served from the host the client reached us at and from localhost; `any` switches the check off.
    origins: Vec<String>,
    any_origin: bool,
    /// Open connections per IP (0 = no limit), and connection attempts per IP per minute (0 = no limit).
    max_per_ip: usize,
    conns_per_min: usize,
}

#[derive(Default)]
struct IpState {
    open: usize,
    recent: VecDeque<Instant>,
}
type IpTable = Arc<Mutex<HashMap<IpAddr, IpState>>>;

/// Holds one of an IP's connection slots until dropped.
struct IpGuard {
    ip: IpAddr,
    table: IpTable,
}
impl Drop for IpGuard {
    fn drop(&mut self) {
        if let Some(s) = self.table.lock().unwrap().get_mut(&self.ip) {
            s.open = s.open.saturating_sub(1);
        }
    }
}

fn admit(table: &IpTable, ip: IpAddr, net: &Net) -> Result<IpGuard, &'static str> {
    let now = Instant::now();
    let mut t = table.lock().unwrap();
    if t.len() > 4096 {
        t.retain(|_, s| s.open > 0 || s.recent.back().is_some_and(|&at| now.duration_since(at) < Duration::from_secs(60)));
    }
    let s = t.entry(ip).or_default();
    while s.recent.front().is_some_and(|&at| now.duration_since(at) >= Duration::from_secs(60)) {
        s.recent.pop_front();
    }
    if net.max_per_ip > 0 && s.open >= net.max_per_ip {
        return Err("too many connections from your address");
    }
    if net.conns_per_min > 0 && s.recent.len() >= net.conns_per_min {
        return Err("too many connection attempts, slow down");
    }
    s.open += 1;
    s.recent.push_back(now);
    Ok(IpGuard { ip, table: table.clone() })
}

/// The address the player really comes from: the peer, or, for a reverse proxy on this machine, the last address in its `X-Forwarded-For`.
/// (Only a loopback peer is believed, so nobody on the internet can claim to be somebody else.)
fn client_ip(peer: IpAddr, req: &Request) -> IpAddr {
    let peer = peer.to_canonical();
    if peer.is_loopback() {
        if let Some(ip) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit(',').next()).and_then(|v| v.trim().parse::<IpAddr>().ok()) {
            return ip.to_canonical();
        }
    }
    peer
}

/// `https://Host.example:8443/x` or `host:1338` -> `host.example` / `host` (lowercase, no scheme, port or path).
fn host_of(s: &str) -> String {
    let s = s.split_once("://").map_or(s, |(_, r)| r);
    let s = s.split('/').next().unwrap_or("");
    let h = if s.starts_with('[') { s.find(']').map_or(s, |i| &s[..=i]) } else { s.rsplit_once(':').map_or(s, |(a, _)| a) };
    h.to_ascii_lowercase()
}

fn is_local_host(h: &str) -> bool {
    h == "localhost" || h == "127.0.0.1" || h == "[::1]" || h.ends_with(".localhost")
}

/// A browser tells which page opens the socket; a page of another site must not be able to drive a visitor's browser into this server.
/// No `Origin` header (bots, tools) is fine: it is a browser's guard, not authentication.
fn origin_ok(req: &Request, net: &Net) -> bool {
    let Some(origin) = req.headers().get("origin") else { return true };
    let Ok(origin) = origin.to_str() else { return false };
    if net.any_origin {
        return true;
    }
    let origin = origin.trim_end_matches('/').to_ascii_lowercase();
    if net.origins.iter().any(|o| *o == origin) {
        return true;
    }
    let oh = host_of(&origin);
    let same_host = req.headers().get("host").and_then(|h| h.to_str().ok()).is_some_and(|h| host_of(h) == oh);
    origin != "null" && (is_local_host(&oh) || same_host)
}

fn refuse(code: u16, why: &str) -> ErrorResponse {
    http::Response::builder().status(code).body(Some(why.to_string())).unwrap()
}

#[tokio::main]
async fn main() {
    let (mut port, mut cfg) = (1338u16, Config { radius: 900.0, max_players: 256, tick_hz: 15, spawns: Some("../game/data/mobspawns.json".into()), weapons: Some("../game/data/weapons.json".into()), pvp: true, npcs: 0, pois: Some("../game/data/pois.json".into()), capitals: Some("../game/data/spawns.json".into()), npc_seed: 1, db: Some("npcs.db".into()), nav: Some("data/nav".into()), npc_time_scale: 1.0, own_weapons: true, body_secs: 7.0, tomb_secs: 3600, loot_secs: 3.0, items: Some("../game/data/items.json".into()), vendors: Some("../game/data/vendors.json".into()), max_speed: 20.0, max_fly_speed: 400.0, ghost_secs: 300.0, logout_secs: 30.0 });
    let mut net = Net { origins: Vec::new(), any_origin: false, max_per_ip: 4, conns_per_min: 30 };
    let (mut accounts_path, mut make_admin, mut set_password) = ("accounts.db".to_string(), None::<String>, None::<String>);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_default();
        match a.as_str() {
            "--port" => port = v.parse().expect(USAGE),
            "--radius" => cfg.radius = v.parse().expect(USAGE),
            "--max-players" => cfg.max_players = v.parse::<usize>().expect(USAGE).clamp(1, 49151),
            "--tick-hz" => cfg.tick_hz = v.parse::<u32>().expect(USAGE).clamp(1, 120),
            "--spawns" => cfg.spawns = (v != "none").then_some(v),
            "--weapons" => cfg.weapons = (v != "none").then_some(v),
            "--pvp" => cfg.pvp = v != "off",
            "--npcs" => cfg.npcs = v.parse().expect(USAGE),
            "--pois" => cfg.pois = (v != "none").then_some(v),
            "--capitals" => cfg.capitals = (v != "none").then_some(v),
            "--npc-seed" => cfg.npc_seed = v.parse().expect(USAGE),
            "--db" => cfg.db = (v != "none").then_some(v),
            "--nav" => cfg.nav = (v != "none").then_some(v),
            "--items" => cfg.items = (v != "none").then_some(v),
            "--vendors" => cfg.vendors = (v != "none").then_some(v),
            "--own-weapons" => cfg.own_weapons = v != "off",
            "--body-secs" => cfg.body_secs = v.parse().expect(USAGE),
            "--tomb-secs" => cfg.tomb_secs = v.parse().expect(USAGE),
            "--loot-secs" => cfg.loot_secs = v.parse().expect(USAGE),
            "--max-per-ip" => net.max_per_ip = v.parse().expect(USAGE),
            "--conns-per-min" => net.conns_per_min = v.parse().expect(USAGE),
            "--origins" => {
                net.any_origin = v == "any";
                net.origins = v.split(',').map(|o| o.trim().trim_end_matches('/').to_ascii_lowercase()).filter(|o| !o.is_empty()).collect();
            }
            "--accounts" => accounts_path = v,
            "--make-admin" => make_admin = Some(v),
            "--set-password" => set_password = Some(v),
            "--max-speed" => cfg.max_speed = v.parse().expect(USAGE),
            "--max-fly-speed" => cfg.max_fly_speed = v.parse().expect(USAGE),
            "--ghost-secs" => cfg.ghost_secs = v.parse().expect(USAGE),
            "--logout-secs" => cfg.logout_secs = v.parse().expect(USAGE),
            "--npc-time-scale" => cfg.npc_time_scale = v.parse().expect(USAGE),
            _ => panic!("{USAGE}"),
        }
    }
    let mut capitals = match &cfg.capitals {
        Some(path) => accounts::load_capitals(path).unwrap_or_else(|e| {
            eprintln!("warning: no racial capitals ({e}): new characters start at the origin");
            Vec::new()
        }),
        None => Vec::new(),
    };
    if let Some(dir) = &cfg.nav {
        match server::nav::Nav::open(dir) {
            Ok(nav) => accounts::ground_capitals(&mut capitals, &nav),
            Err(e) => eprintln!("no nav, newcomers appear on the bindstones themselves: {e}"),
        }
    }
    let accounts = Accounts::open(&accounts_path, capitals).unwrap_or_else(|e| panic!("accounts database {accounts_path}: {e}"));
    if let Some(user) = make_admin {
        println!("{}", if accounts.make_admin(&user, true) { format!("{user} is an admin now") } else { format!("no account named {user}") });
        return;
    }
    if let Some(arg) = set_password {
        let (user, pw) = arg.split_once(':').expect(USAGE);
        match accounts.set_password(user, pw) {
            Ok(true) => println!("password of {user} changed"),
            Ok(false) => println!("no account named {user}"),
            Err(e) => println!("{e}"),
        }
        return;
    }
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>(8192);
    let mut world = tokio::spawn(world::run(cfg.clone(), cmd_rx));
    let (net, ips) = (Arc::new(net), IpTable::default());
    let listener = TcpListener::bind(("0.0.0.0", port)).await.expect("bind");
    println!("OpenOctober server on ws://0.0.0.0:{port}  (radius {} m, {} Hz, max {} players)", cfg.radius, cfg.tick_hz, cfg.max_players);
    loop {
        tokio::select! {
            _ = &mut world => break, // the world task ended (Ctrl-C): it has saved the NPCs
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { continue };
                let _ = stream.set_nodelay(true);
                tokio::spawn(connection(stream, peer, cmd_tx.clone(), net.clone(), ips.clone(), accounts.clone()));
            }
        }
    }
}

async fn connection(stream: TcpStream, peer: SocketAddr, cmds: mpsc::Sender<Cmd>, net: Arc<Net>, ips: IpTable, accounts: Arc<Accounts>) {
    let mut wsc = WebSocketConfig::default();
    wsc.max_message_size = Some(4096);
    wsc.max_frame_size = Some(4096);
    // the handshake decides whether we want this client at all: right page, not too many connections from its address
    let admitted: Arc<Mutex<Option<(Option<IpGuard>, bool, IpAddr)>>> = Arc::default();
    let slot = admitted.clone();
    let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if !origin_ok(req, &net) {
            return Err(refuse(403, "origin not allowed"));
        }
        let ip = client_ip(peer.ip(), req);
        let local = ip.is_loopback();
        let guard = if local { None } else { Some(admit(&ips, ip, &net).map_err(|why| refuse(429, why))?) };
        *slot.lock().unwrap() = Some((guard, local, ip));
        Ok(resp)
    };
    let Ok(Ok(ws)) = tokio::time::timeout(Duration::from_secs(HANDSHAKE_SECS), tokio_tungstenite::accept_hdr_async_with_config(stream, check, Some(wsc))).await else { return };
    let Some((_slot, local, ip)) = admitted.lock().unwrap().take() else { return }; // _slot: the IP's connection slot, released when this connection ends
    let (mut sink, mut source) = ws.split();

    let (tx, mut rx) = mpsc::channel::<Message>(128);
    let mut writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    });
    let say = |v: serde_json::Value| tx.try_send(Message::Text(v.to_string()));

    // ---- logging in: sign up / log in as often as needed, then `enter` once the client has loaded the world
    let (mut session, mut auth_errors): (Option<(accounts::Account, SessionGuard)>, u32) = (None, 0);
    let admin = loop {
        let m = tokio::select! {
            _ = &mut writer => return,
            _ = tokio::time::sleep(PRE_ENTER_IDLE) => return,
            _ = async { match session.as_mut() { Some((_, g)) => { let _ = (&mut g.kicked).await; } None => std::future::pending().await } } => {
                let _ = say(serde_json::json!({ "t": "autherr", "msg": "you logged in somewhere else" }));
                return;
            }
            m = source.next() => m,
        };
        let Some(Ok(Message::Text(t))) = m else {
            match m {
                Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return,
            }
        };
        let fail = |msg: &str| serde_json::json!({ "t": "autherr", "msg": msg });
        match serde_json::from_str::<ClientMsg>(&t) {
            Ok(ClientMsg::Register { username, email, password, name, race, gender, gear }) if session.is_none() => {
                let reply = if !accounts.ip_allowed(ip) {
                    Err("too many attempts from your address, wait a minute")
                } else {
                    accounts.register(SignUp { username, email, password, profile: Profile { name, race, gender, gear } }).await
                };
                match reply {
                    Ok(()) => drop(say(serde_json::json!({ "t": "registered" }))),
                    Err(e) => {
                        auth_errors += 1;
                        drop(say(fail(e)));
                    }
                }
            }
            Ok(ClientMsg::Login { username, password }) if session.is_none() => {
                let reply = if !accounts.ip_allowed(ip) { Err("too many attempts from your address, wait a minute") } else { accounts.login(&username, &password).await };
                match reply {
                    Ok((acc, guard)) => {
                        let (pos, capital) = (accounts.spawn_of(&acc), accounts.capital_of(&acc.profile.race));
                        let p = &acc.profile;
                        let _ = say(serde_json::json!({ "t": "auth", "username": acc.username, "admin": acc.admin, "name": p.name, "race": p.race, "gender": p.gender, "gear": p.gear, "pos": pos, "capital": capital, "fresh": acc.pos.is_none() }));
                        session = Some((acc, guard));
                    }
                    Err(e) => {
                        auth_errors += 1;
                        drop(say(fail(e)));
                    }
                }
            }
            Ok(ClientMsg::Enter { admin }) => match &session {
                Some((acc, _)) if admin && !acc.admin => drop(say(fail("not an admin account"))),
                Some(_) => break admin,
                None => drop(say(fail("log in first"))),
            },
            _ => {} // nothing else is understood before entering
        }
        if auth_errors >= MAX_AUTH_ERRORS {
            println!("- {ip}: {auth_errors} refused log-ins / sign-ups: dropped");
            return;
        }
    };
    let Some((acc, mut guard)) = session else { return };

    // ---- entering: the character joins the world, at its saved place (a new one at its race's capital)
    let (reply, reply_rx) = oneshot::channel();
    if cmds.send(Cmd::Join(JoinReq { account: Some(acc.id), name: acc.profile.name.clone(), race: acc.profile.race.clone(), gender: acc.profile.gender.clone(), gear: acc.profile.gear.clone(), tx: tx.clone(), reply })).await.is_err() {
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
    if (local || admin) && cmds.send(Cmd::Trust(id)).await.is_err() {
        return;
    }
    if !admin {
        let [x, y, z, yaw] = accounts.spawn_of(&acc);
        if cmds.send(Cmd::Place(id, PosUpdate { x, y, z, yaw, speed: 0, flags: 0 })).await.is_err() {
            return;
        }
    }
    // where the character is now goes to the database (an admin looking around does not move his character; a dead one gets up at the capital)
    let save_pos = |cmds: mpsc::Sender<Cmd>, accounts: Arc<Accounts>, account: i64, race: String| async move {
        let (t, r) = oneshot::channel();
        if cmds.send(Cmd::Where(id, t)).await.is_ok() {
            if let Ok(Some((p, dead))) = r.await {
                accounts.save_pos(account, if dead { accounts.capital_of(&race) } else { p });
            }
        }
    };
    let mut autosave = tokio::time::interval(Duration::from_secs(POS_SAVE_SECS));
    autosave.tick().await;

    let (mut bucket, mut refilled, mut over) = (MSG_BURST, Instant::now(), 0u32);
    let mut last_active = Instant::now();
    loop {
        tokio::select! {
            _ = &mut writer => break, // kicked, or the socket died
            _ = &mut guard.kicked => break, // the account logged in again somewhere else
            _ = autosave.tick(), if !admin => save_pos(cmds.clone(), accounts.clone(), acc.id, acc.profile.race.clone()).await,
            _ = tokio::time::sleep_until(last_active + IDLE_KICK) => {
                println!("- id {id} sent nothing for {} min: kicked", IDLE_KICK.as_secs() / 60);
                break;
            }
            m = source.next() => match m {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                // a flood of frames: drop what is over the rate, and kick a client that keeps on
                Some(Ok(_)) if {
                    let now = Instant::now();
                    bucket = (bucket + now.duration_since(refilled).as_secs_f32() * MSG_RATE).min(MSG_BURST);
                    refilled = now;
                    let empty = bucket < 1.0;
                    if empty { over += 1 } else { bucket -= 1.0; if bucket > MSG_BURST / 2.0 { over = 0 } }
                    empty
                } => {
                    if over > MSG_MAX_OVER {
                        println!("- id {id} flooding: kicked");
                        break;
                    }
                }
                Some(Ok(Message::Binary(b))) => {
                    if let Some(mut p) = PosUpdate::decode_frame(&b) {
                        if !admin { p.flags &= !FLAG_FLY; } // free flight is for admins
                        last_active = Instant::now();
                        if cmds.send(Cmd::Pos(id, p)).await.is_err() { break; }
                    }
                }
                Some(Ok(Message::Text(t))) => {
                    let cmd = match serde_json::from_str::<ClientMsg>(&t) {
                        Ok(ClientMsg::Chat { text }) => Some(Cmd::Chat(id, text)),
                        Ok(ClientMsg::Equip { w, drawn }) => Some(Cmd::Equip(id, w, drawn)),
                        Ok(ClientMsg::Attack { kind, clip }) => Some(Cmd::Attack(id, kind, clip)),
                        Ok(ClientMsg::Block { on }) => Some(Cmd::Block(id, on)),
                        Ok(ClientMsg::Bow { what }) => Some(Cmd::Bow(id, what)),
                        Ok(ClientMsg::Shoot { dir, from, power, spell }) => Some(Cmd::Shoot(id, from, dir, power, spell)),
                        Ok(ClientMsg::Respawn) => Some(Cmd::Respawn(id)),
                        Ok(ClientMsg::Loot { cid }) => Some(Cmd::Loot(id, cid)),
                        Ok(ClientMsg::OpenGrave { cid }) => Some(Cmd::OpenGrave(id, cid)),
                        Ok(ClientMsg::Take { cid, item, qty }) => Some(Cmd::Take(id, cid, item, qty)),
                        Ok(ClientMsg::Shop { npc }) => Some(Cmd::Shop(id, npc)),
                        Ok(ClientMsg::Buy { npc, item, qty }) => Some(Cmd::Buy(id, npc, item, qty)),
                        Ok(ClientMsg::Sell { npc, item, qty }) => Some(Cmd::Sell(id, npc, item, qty)),
                        _ => None,
                    };
                    if let Some(c) = cmd {
                        last_active = Instant::now();
                        if cmds.send(c).await.is_err() { break; }
                    }
                }
                _ => {}
            },
        }
    }
    // the world first: it keeps the body standing, or takes it away, and either way can still tell where it was
    let _ = cmds.send(Cmd::Leave(id)).await;
    if !admin {
        save_pos(cmds.clone(), accounts.clone(), acc.id, acc.profile.race.clone()).await;
    }
    drop(guard); // only now may a newer login of this account go on
}
