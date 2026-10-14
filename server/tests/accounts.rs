//! Accounts through the real binary: sign-up rules, log-in, spawn at the racial capital, the saved place, one session per account, admins.
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

struct Server {
    child: Child,
    port: u16,
    db: String,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn serve(tag: &str) -> Server {
    serve_with(tag, &[]).await
}

async fn serve_with(tag: &str, extra: &[&str]) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let db = std::env::temp_dir().join(format!("df_acc_{tag}_{port}.db")).to_string_lossy().to_string();
    let _ = std::fs::remove_file(&db);
    let caps = std::env::temp_dir().join(format!("df_acc_caps_{tag}_{port}.json"));
    std::fs::write(&caps, r#"[{"name":"Sanguine","race":"Human","x":100,"y":50,"z":200},{"name":"Shoal","race":"Alfar","x":-300,"y":60,"z":-400}]"#).unwrap();
    let (port_s, caps_s) = (port.to_string(), caps.to_string_lossy().to_string());
    let mut args = vec!["--port", &port_s, "--spawns", "none", "--weapons", "none", "--npcs", "0", "--db", "none", "--accounts", &db, "--nav", "none", "--items", "none", "--vendors", "none", "--pois", "none", "--capitals", &caps_s];
    args.extend_from_slice(extra);
    let child = Command::new(env!("CARGO_BIN_EXE_server")).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Server { child, port, db }
}

async fn connect(s: &Server) -> Ws {
    tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", s.port)).await.unwrap().0
}

/// Send `msg`, return the first JSON reply whose `t` is one of `until`.
async fn ask(ws: &mut Ws, msg: Value, until: &[&str]) -> Value {
    ws.send(Message::Text(msg.to_string())).await.unwrap();
    expect(ws, until).await
}

async fn expect(ws: &mut Ws, until: &[&str]) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                if until.contains(&v["t"].as_str().unwrap_or("")) {
                    return v;
                }
            }
            Ok(Some(Ok(_))) => {}
            other => panic!("waiting for {until:?}: {other:?}"),
        }
    }
}

fn signup(user: &str, name: &str, race: &str) -> Value {
    json!({ "t": "register", "username": user, "email": format!("{user}@example.com"), "password": "correct horse", "name": name, "race": race, "gender": "m" })
}

fn login(user: &str, pw: &str) -> Value {
    json!({ "t": "login", "username": user, "password": pw })
}

async fn register_ok(ws: &mut Ws, user: &str, name: &str, race: &str) {
    assert_eq!(ask(ws, signup(user, name, race), &["registered", "autherr"]).await["t"], "registered");
}

fn pos_frame(x: f32, y: f32, z: f32) -> Message {
    let mut b = vec![1u8];
    for v in [x, y, z, 0.0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0]);
    Message::Binary(b)
}

#[tokio::test]
async fn sign_up_rules() {
    let s = serve("rules").await;
    let mut ws = connect(&s).await;
    let bad = |mut v: Value, k: &str, to: Value| {
        v[k] = to;
        v
    };
    for (v, why) in [
        (bad(signup("ab", "Hero", "hum"), "username", json!("ab")), "short username"),
        (bad(signup("has space", "Hero", "hum"), "username", json!("has space")), "space in username"),
        (bad(signup("okuser", "Hero", "hum"), "email", json!("nope")), "email"),
        (bad(signup("okuser", "Hero", "hum"), "password", json!("short")), "short password"),
        (bad(signup("okuser", "Hero", "hum"), "name", json!("X")), "short name"),
        (bad(signup("okuser", "Hero", "hum"), "name", json!("<b>x</b>")), "markup in name"),
        (signup("okuser", "Hero", "elf"), "unknown race"),
    ] {
        assert_eq!(ask(&mut ws, v, &["registered", "autherr"]).await["t"], "autherr", "{why}");
    }
    register_ok(&mut ws, "Anna", "Anna the Bold", "hum").await;
    // names are unique whatever the case
    assert!(ask(&mut ws, signup("anna", "Other", "hum"), &["registered", "autherr"]).await["msg"].as_str().unwrap().contains("username"));
    let mut same_email = signup("bertil", "Bertil", "hum");
    same_email["email"] = json!("ANNA@example.com");
    assert!(ask(&mut ws, same_email, &["registered", "autherr"]).await["msg"].as_str().unwrap().contains("email"));
    assert!(ask(&mut ws, signup("bertil", "anna the bold", "hum"), &["registered", "autherr"]).await["msg"].as_str().unwrap().contains("character"));
    // the password is not stored in the clear
    let raw = std::fs::read(&s.db).unwrap();
    assert!(!raw.windows(13).any(|w| w == b"correct horse"));
}

#[tokio::test]
async fn wrong_passwords_say_nothing_and_lock_the_name() {
    let s = serve("lock").await;
    let mut ws = connect(&s).await;
    register_ok(&mut ws, "carin", "Carin", "hum").await;
    let wrong_pw = ask(&mut ws, login("carin", "not the password"), &["auth", "autherr"]).await;
    let wrong_user = ask(&mut ws, login("nobody", "not the password"), &["auth", "autherr"]).await;
    assert_eq!(wrong_pw["t"], "autherr");
    assert_eq!(wrong_pw["msg"], wrong_user["msg"], "no hint which half was wrong");
    for _ in 0..3 {
        ask(&mut ws, login("carin", "still wrong"), &["autherr"]).await;
    }
    // five failures: even the right password is refused for a while
    ask(&mut ws, login("carin", "wrong again"), &["autherr"]).await;
    let locked = ask(&mut ws, login("carin", "correct horse"), &["auth", "autherr"]).await;
    assert_eq!(locked["t"], "autherr");
    assert!(locked["msg"].as_str().unwrap().contains("minute"));
}

#[tokio::test]
async fn a_new_character_starts_at_its_capital_and_comes_back_where_it_left() {
    let s = serve("spawn").await;
    let mut ws = connect(&s).await;
    register_ok(&mut ws, "dagny", "Dagny", "alf").await;
    let a = ask(&mut ws, login("dagny", "correct horse"), &["auth", "autherr"]).await;
    assert_eq!(a["t"], "auth");
    assert_eq!(a["fresh"], true);
    assert_eq!((a["name"].as_str(), a["race"].as_str(), a["admin"].as_bool()), (Some("Dagny"), Some("alf"), Some(false)));
    // wire x / z: game z is negated; the capital is stood on, a metre above its height
    assert_eq!(a["pos"], json!([-300.0, 61.0, 400.0, 0.0]));
    assert_eq!(a["pos"], a["capital"]);

    let w = ask(&mut ws, json!({ "t": "enter" }), &["welcome"]).await;
    assert!(w["players"].as_array().unwrap().iter().any(|p| p["name"] == "Dagny" && p["race"] == "alf"));
    ws.send(pos_frame(-250.0, 70.0, 450.0)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    ws.close(None).await.unwrap();

    let mut ws = connect(&s).await;
    let b = ask(&mut ws, login("dagny", "correct horse"), &["auth", "autherr"]).await;
    assert_eq!(b["fresh"], false);
    assert_eq!(b["pos"], json!([-250.0, 70.0, 450.0, 0.0]));
    assert_eq!(b["capital"], json!([-300.0, 61.0, 400.0, 0.0]));
}

#[tokio::test]
async fn nothing_works_before_logging_in() {
    let s = serve("gate").await;
    let mut ws = connect(&s).await;
    let r = ask(&mut ws, json!({ "t": "enter" }), &["welcome", "autherr"]).await;
    assert_eq!(r["t"], "autherr");
    // chat, positions and the old anonymous join are all ignored
    ws.send(Message::Text(json!({ "t": "chat", "text": "hi" }).to_string())).await.unwrap();
    ws.send(Message::Text(json!({ "t": "join", "name": "x", "race": "hum", "gender": "m" }).to_string())).await.unwrap();
    ws.send(pos_frame(1.0, 2.0, 3.0)).await.unwrap();
    register_ok(&mut ws, "elin", "Elin", "hum").await;
    // still alive and able to log in properly
    assert_eq!(ask(&mut ws, login("elin", "correct horse"), &["auth", "autherr"]).await["t"], "auth");
}

#[tokio::test]
async fn logging_in_again_replaces_the_old_session() {
    let s = serve("dup").await;
    let mut first = connect(&s).await;
    register_ok(&mut first, "frida", "Frida", "hum").await;
    ask(&mut first, login("frida", "correct horse"), &["auth"]).await;
    ask(&mut first, json!({ "t": "enter" }), &["welcome"]).await;
    first.send(pos_frame(120.0, 55.0, -180.0)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut second = connect(&s).await;
    // the second login waits for the first to be saved, so it sees where she was
    let a = ask(&mut second, login("frida", "correct horse"), &["auth", "autherr"]).await;
    assert_eq!(a["pos"], json!([120.0, 55.0, -180.0, 0.0]));
    // the first connection is closed
    loop {
        match tokio::time::timeout(Duration::from_secs(5), first.next()).await {
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(_))) => {}
            Err(_) => panic!("the old session is still open"),
        }
    }
    let w = ask(&mut second, json!({ "t": "enter" }), &["welcome"]).await;
    assert_eq!(w["players"].as_array().unwrap().iter().filter(|p| p["name"] == "Frida").count(), 1);
}

#[tokio::test]
async fn only_admins_get_the_free_fly_session() {
    let s = serve("admin").await;
    let mut ws = connect(&s).await;
    register_ok(&mut ws, "gunnar", "Gunnar", "hum").await;
    ask(&mut ws, login("gunnar", "correct horse"), &["auth"]).await;
    assert_eq!(ask(&mut ws, json!({ "t": "enter", "admin": true }), &["welcome", "autherr"]).await["t"], "autherr");
    ws.close(None).await.unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_server")).args(["--accounts", &s.db, "--capitals", "none", "--make-admin", "gunnar"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("admin now"));

    let mut ws = connect(&s).await;
    let a = ask(&mut ws, login("gunnar", "correct horse"), &["auth"]).await;
    assert_eq!(a["admin"], true);
    assert_eq!(ask(&mut ws, json!({ "t": "enter", "admin": true }), &["welcome", "autherr"]).await["t"], "welcome");
    // flying around as an admin does not move the character
    ws.send(pos_frame(5000.0, 400.0, 5000.0)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    ws.close(None).await.unwrap();
    let mut ws = connect(&s).await;
    let b = ask(&mut ws, login("gunnar", "correct horse"), &["auth"]).await;
    assert_eq!(b["fresh"], true);
}

#[test]
fn a_capital_that_is_a_solid_stone_becomes_open_ground_beside_it_facing_it() {
    use server::accounts::{ground_capitals, Capital};
    use server::nav::{Nav, NavTile, F_WALL};
    // flat ground at 100 m with a blocked block of 5x5 cells (the bindstone) around cell (50, 50), i.e. around x = z = 101
    let blocked: Vec<(usize, usize, u8)> = (48..=52).flat_map(|x| (48..=52).map(move |z| (x, z, F_WALL))).collect();
    let nav = Nav::from_tiles(vec![NavTile::flat(0, 0, 100.0, &blocked)]);
    // a region of 1500 free cells is "open ground": the flat tile has far more
    let mut caps = vec![Capital { race: "hum".into(), x: 101.0, y: 300.0, z: 101.0, yaw: 0.0 }, Capital { race: "ork".into(), x: 90_000.0, y: 5.0, z: 90_000.0, yaw: 0.0 }];
    ground_capitals(&mut caps, &nav);
    let c = &caps[0];
    let d = ((c.x - 101.0).powi(2) + (c.z - 101.0).powi(2)).sqrt();
    assert!((6.0..=14.0).contains(&d), "stands a few metres from the stone, not in it: {d}");
    assert!(nav.walkable(c.x, c.z));
    assert!((c.y - 100.0).abs() < 0.1, "at the terrain height there: {}", c.y);
    // forward is (-sin yaw, -cos yaw): it points at the stone
    let (fx, fz) = (-c.yaw.sin(), -c.yaw.cos());
    assert!(fx * (101.0 - c.x) + fz * (101.0 - c.z) > 0.99 * d);
    // off the baked map nothing changes
    assert_eq!((caps[1].x, caps[1].y, caps[1].z), (90_000.0, 5.0, 90_000.0));
}

/// Log in and enter; the welcome says which id the character has.
async fn enter(s: &Server, user: &str) -> (Ws, u64) {
    let mut ws = connect(s).await;
    assert_eq!(ask(&mut ws, login(user, "correct horse"), &["auth", "autherr"]).await["t"], "auth");
    let w = ask(&mut ws, json!({ "t": "enter" }), &["welcome"]).await;
    (ws, w["id"].as_u64().unwrap())
}

#[tokio::test]
async fn a_lost_connection_leaves_the_body_standing_and_logout_is_a_wait() {
    let s = serve_with("ghost", &["--ghost-secs", "4", "--logout-secs", "2"]).await;
    let mut a = connect(&s).await;
    register_ok(&mut a, "watcher", "Watcher", "hum").await;
    register_ok(&mut a, "victim", "Victim", "hum").await;
    drop(a);
    let (mut a, _) = enter(&s, "watcher").await;
    let (mut b, bid) = enter(&s, "victim").await;
    b.send(pos_frame(101.0, 51.0, 201.0)).await.unwrap();
    a.send(pos_frame(100.0, 51.0, 200.0)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // the connection just dies: nobody sees him leave, and his body can be taken over again within the wait
    drop(b);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (mut b, bid2) = enter(&s, "victim").await;
    assert_eq!(bid, bid2, "he is back in his own body");
    b.send(pos_frame(101.0, 51.0, 201.0)).await.unwrap();

    // /logout: everybody is told, moving cancels it
    b.send(Message::Text(json!({ "t": "chat", "text": "/logout" }).to_string())).await.unwrap();
    let m = expect(&mut a, &["logout", "left"]).await;
    assert_eq!((m["t"].as_str(), m["id"].as_u64()), (Some("logout"), Some(bid)));
    b.send(pos_frame(105.0, 51.0, 201.0)).await.unwrap();
    assert_eq!(expect(&mut a, &["logoutcancel", "left"]).await["t"], "logoutcancel");
    // again, standing still: when the time is up he is out of the world, and told so
    b.send(Message::Text(json!({ "t": "chat", "text": "/logout" }).to_string())).await.unwrap();
    assert_eq!(expect(&mut a, &["logout", "left"]).await["t"], "logout");
    assert_eq!(expect(&mut b, &["loggedout"]).await["t"], "loggedout");
    assert_eq!(expect(&mut a, &["left"]).await["id"].as_u64(), Some(bid));

    // a plain disconnect: the body stays for ghost-secs, then is gone
    let (mut b, bid3) = enter(&s, "victim").await;
    b.send(pos_frame(101.0, 51.0, 201.0)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(b);
    let t0 = std::time::Instant::now();
    assert_eq!(expect(&mut a, &["left"]).await["id"].as_u64(), Some(bid3));
    assert!(t0.elapsed() > Duration::from_secs(3), "gone only after the wait: {:?}", t0.elapsed());
}
