//! Accounts: username + email + password, one character per account (name, race, gender, armour) and the place it was last seen.
//! Stored in SQLite (`--accounts accounts.db`), passwords as argon2 hashes. The connection task of `main.rs` runs the login / sign-up
//! handshake against this before anything reaches the world task; the world never sees a password.
//!
//! * **Spawn:** a character that has never been in the world starts at the capital of its race (`spawns.json`, `--capitals`); after that, where it was last
//!   (saved every `SAVE_EVERY` seconds, and when the connection ends).
//! * **One session per account:** logging in again kicks the older session and waits (a few seconds at most) until it has been saved and removed.
//! * **Abuse:** five wrong passwords lock the username for a minute; an address (not the local machine) gets `IP_ATTEMPTS` attempts per minute.
//!   A wrong username and a wrong password get the same answer, and cost the same time.
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::{oneshot, Semaphore};
use tokio::time::Instant;

use crate::nav::Nav;
use crate::proto::{clean, GENDERS, MAX_NAME, RACES};

pub const MIN_PASSWORD: usize = 8;
pub const MAX_PASSWORD: usize = 128;
const FAILS_BEFORE_LOCK: u32 = 5;
const LOCK_SECS: u64 = 60;
pub const IP_ATTEMPTS: usize = 20;
/// How long a new login waits for the session it replaces to be saved and removed.
const TAKEOVER_WAIT: Duration = Duration::from_secs(5);

/// A racial capital: where a new character of `race` (a code from `RACES`) starts. Wire coordinates (z already negated).
#[derive(Clone, Debug)]
pub struct Capital {
    pub race: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Which way a newcomer faces (towards the bindstone).
    pub yaw: f32,
}

/// The capitals are the bindstones themselves (solid objects): move each spawn to open ground a few metres away, at the terrain height there, facing the stone.
/// Capitals the nav map does not cover keep the bindstone's own spot.
pub fn ground_capitals(caps: &mut [Capital], nav: &Nav) {
    for c in caps {
        let at = (c.x, c.z);
        let ring = (0..8).map(|i| i as f32 * std::f32::consts::FRAC_PI_4).map(|a| (at.0 + 8.0 * a.cos(), at.1 + 8.0 * a.sin()));
        let spot = ring.filter_map(|p| nav.snap_open(p, 6, 1500).filter(|s| (s.0 - p.0).hypot(s.1 - p.1) < 3.0)).next().or_else(|| nav.snap_open(at, 40, 1500));
        let Some((x, z)) = spot else { continue };
        c.yaw = (at.0 - x).atan2(at.1 - z) + std::f32::consts::PI; // forward is (-sin yaw, -cos yaw)
        (c.x, c.z) = (x, z);
        if let Some(h) = nav.height(x, z) {
            c.y = h;
        }
    }
}

/// Read the capitals out of `game-viewer/data/spawns.json` (`[{"name","race":"Alfar",...,"x","y","z"}]`, game z).
pub fn load_capitals(path: &str) -> Result<Vec<Capital>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let code = |name: &str| match name.to_ascii_lowercase().as_str() {
        "human" => Some("hum"),
        "alfar" => Some("alf"),
        "dwarf" => Some("dwf"),
        "ork" => Some("ork"),
        "mirdain" => Some("mir"),
        "mahirim" => Some("mhm"),
        _ => None,
    };
    Ok(rows
        .iter()
        .filter_map(|r| {
            let race = code(r.get("race")?.as_str()?)?;
            let f = |k: &str| r.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
            Some(Capital { race: race.to_string(), x: f("x")?, y: f("y")?, z: -f("z")?, yaw: 0.0 })
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub race: String,
    pub gender: String,
    pub gear: String,
}

#[derive(Clone, Debug)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub admin: bool,
    pub profile: Profile,
    /// Where the character was last seen (x, y, z, yaw); `None` = never entered the world.
    pub pos: Option<[f32; 4]>,
}

pub struct SignUp {
    pub username: String,
    pub email: String,
    pub password: String,
    pub profile: Profile,
}

struct Session {
    token: u64,
    kick: oneshot::Sender<()>,
    done: oneshot::Receiver<()>,
}

/// Held by the connection of a logged-in account. `kicked` resolves when a newer login replaces this session; drop the guard (after saving)
/// to let that login go on.
pub struct SessionGuard {
    accounts: Arc<Accounts>,
    pub id: i64,
    token: u64,
    pub kicked: oneshot::Receiver<()>,
    _done: oneshot::Sender<()>,
}
impl Drop for SessionGuard {
    fn drop(&mut self) {
        let mut s = self.accounts.sessions.lock().unwrap();
        if s.get(&self.id).is_some_and(|x| x.token == self.token) {
            s.remove(&self.id);
        }
    }
}

pub struct Accounts {
    conn: Mutex<Connection>,
    capitals: Vec<Capital>,
    fails: Mutex<HashMap<String, (u32, Instant)>>,
    ips: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    sessions: Mutex<HashMap<i64, Session>>,
    next_token: Mutex<u64>,
    hashing: Semaphore,
    /// Verified against when the username is unknown, so that a wrong username costs the same as a wrong password.
    dummy: String,
}

fn hash_password(pw: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(pw.as_bytes(), &salt).expect("argon2").to_string()
}

fn check_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
}

fn valid_username(s: &str) -> bool {
    (3..=20).contains(&s.chars().count()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn valid_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    s.len() <= 100 && !local.is_empty() && !domain.contains('@') && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && s.chars().all(|c| c.is_ascii_graphic())
}

fn valid_char_name(s: &str) -> bool {
    let n = s.chars().count();
    (2..=MAX_NAME).contains(&n) && s.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '\'' | '-')) && s.chars().any(|c| c.is_alphabetic())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64
}

impl Accounts {
    pub fn open(path: &str, capitals: Vec<Capital>) -> rusqlite::Result<Arc<Self>> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
                id INTEGER PRIMARY KEY, username TEXT NOT NULL UNIQUE COLLATE NOCASE, email TEXT NOT NULL UNIQUE COLLATE NOCASE,
                pw_hash TEXT NOT NULL, admin INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL,
                char_name TEXT NOT NULL UNIQUE COLLATE NOCASE, race TEXT NOT NULL, gender TEXT NOT NULL, gear TEXT NOT NULL DEFAULT '',
                x REAL, y REAL, z REAL, yaw REAL, last_login INTEGER);",
        )?;
        Ok(Arc::new(Accounts {
            conn: Mutex::new(conn),
            capitals,
            fails: Mutex::default(),
            ips: Mutex::default(),
            sessions: Mutex::default(),
            next_token: Mutex::new(1),
            hashing: Semaphore::new(4),
            dummy: hash_password("not a password anybody has"),
        }))
    }

    /// May this address make another sign-up / login attempt (counted)? The local machine is never limited.
    pub fn ip_allowed(&self, ip: IpAddr) -> bool {
        if ip.is_loopback() {
            return true;
        }
        let now = Instant::now();
        let mut t = self.ips.lock().unwrap();
        if t.len() > 4096 {
            t.retain(|_, q| q.back().is_some_and(|&at| now.duration_since(at) < Duration::from_secs(60)));
        }
        let q = t.entry(ip).or_default();
        while q.front().is_some_and(|&at| now.duration_since(at) >= Duration::from_secs(60)) {
            q.pop_front();
        }
        if q.len() >= IP_ATTEMPTS {
            return false;
        }
        q.push_back(now);
        true
    }

    pub fn make_admin(&self, username: &str, admin: bool) -> bool {
        self.conn.lock().unwrap().execute("UPDATE accounts SET admin = ?1 WHERE username = ?2", params![admin as i64, username]).is_ok_and(|n| n > 0)
    }

    pub fn set_password(&self, username: &str, password: &str) -> Result<bool, &'static str> {
        if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&password.chars().count()) {
            return Err("password must be 8 to 128 characters");
        }
        let h = hash_password(password);
        Ok(self.conn.lock().unwrap().execute("UPDATE accounts SET pw_hash = ?1 WHERE username = ?2", params![h, username]).is_ok_and(|n| n > 0))
    }

    /// Create an account with its character. The error is meant to be shown to the player.
    pub async fn register(self: &Arc<Self>, req: SignUp) -> Result<(), &'static str> {
        let (username, email) = (req.username.trim().to_string(), req.email.trim().to_string());
        let p = &req.profile;
        let name = clean(&p.name, MAX_NAME);
        if !valid_username(&username) {
            return Err("username: 3 to 20 letters, digits, _ - .");
        }
        if !valid_email(&email) {
            return Err("that does not look like an email address");
        }
        if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&req.password.chars().count()) {
            return Err("password must be 8 to 128 characters");
        }
        if !valid_char_name(&name) {
            return Err("character name: 2 to 16 letters, digits, spaces, ' -");
        }
        if !RACES.contains(&p.race.as_str()) || !GENDERS.contains(&p.gender.as_str()) {
            return Err("unknown race or gender");
        }
        {
            let c = self.conn.lock().unwrap();
            let taken = |col: &str, v: &str| c.query_row(&format!("SELECT 1 FROM accounts WHERE {col} = ?1"), [v], |_| Ok(())).optional().ok().flatten().is_some();
            if taken("username", &username) {
                return Err("username is taken");
            }
            if taken("email", &email) {
                return Err("email is already registered");
            }
            if taken("char_name", &name) {
                return Err("character name is taken");
            }
        }
        let pw = req.password;
        let permit = self.hashing.acquire().await.map_err(|_| "server busy")?;
        let hash = tokio::task::spawn_blocking(move || hash_password(&pw)).await.map_err(|_| "server busy")?;
        drop(permit);
        let gear = String::new(); // everybody starts naked: armour is not chosen at sign-up
        // the unique indexes decide a race between two sign-ups
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO accounts (username, email, pw_hash, created, char_name, race, gender, gear) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![username, email, hash, now_secs(), name, p.race, p.gender, gear],
            )
            .map(|_| ())
            .map_err(|_| "username, email or character name is taken")
    }

    /// Check the password and, if right, take over the account's session (kicking an older one). Every failure gets the same answer.
    pub async fn login(self: &Arc<Self>, username: &str, password: &str) -> Result<(Account, SessionGuard), &'static str> {
        const WRONG: &str = "wrong username or password";
        let username = username.trim();
        let key = username.to_ascii_lowercase();
        if let Some((n, until)) = self.fails.lock().unwrap().get(&key) {
            if *n >= FAILS_BEFORE_LOCK && Instant::now() < *until {
                return Err("too many wrong passwords, try again in a minute");
            }
        }
        let row: Option<(i64, String)> = self.conn.lock().unwrap().query_row("SELECT id, pw_hash FROM accounts WHERE username = ?1", [username], |r| Ok((r.get(0)?, r.get(1)?))).optional().ok().flatten();
        let (hash, id) = match &row {
            Some((id, h)) => (h.clone(), Some(*id)),
            None => (self.dummy.clone(), None),
        };
        let pw: String = password.chars().take(MAX_PASSWORD).collect();
        let permit = self.hashing.acquire().await.map_err(|_| "server busy")?;
        let ok = tokio::task::spawn_blocking(move || check_password(&pw, &hash)).await.map_err(|_| "server busy")?;
        drop(permit);
        let Some(id) = id.filter(|_| ok) else {
            let mut f = self.fails.lock().unwrap();
            if f.len() > 4096 {
                let now = Instant::now();
                f.retain(|_, (_, until)| *until > now);
            }
            let e = f.entry(key).or_insert((0, Instant::now()));
            if e.0 >= FAILS_BEFORE_LOCK && Instant::now() >= e.1 {
                e.0 = 0; // the lock ran out
            }
            e.0 += 1;
            e.1 = Instant::now() + Duration::from_secs(LOCK_SECS);
            return Err(WRONG);
        };
        self.fails.lock().unwrap().remove(&key);

        // an older session of this account goes first: it saves where the character was, and only then do we read it
        let old = self.sessions.lock().unwrap().remove(&id);
        if let Some(old) = old {
            let _ = old.kick.send(());
            let _ = tokio::time::timeout(TAKEOVER_WAIT, old.done).await;
        }
        let token = {
            let mut t = self.next_token.lock().unwrap();
            *t += 1;
            *t
        };
        let (kick, kicked) = oneshot::channel();
        let (done_tx, done) = oneshot::channel();
        self.sessions.lock().unwrap().insert(id, Session { token, kick, done });
        let guard = SessionGuard { accounts: self.clone(), id, token, kicked, _done: done_tx };
        let acc = self.get(id).ok_or(WRONG)?;
        let _ = self.conn.lock().unwrap().execute("UPDATE accounts SET last_login = ?1 WHERE id = ?2", params![now_secs(), id]);
        Ok((acc, guard))
    }

    pub fn get(&self, id: i64) -> Option<Account> {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT username, admin, char_name, race, gender, gear, x, y, z, yaw FROM accounts WHERE id = ?1", [id], |r| {
                let pos = match (r.get::<_, Option<f32>>(6)?, r.get::<_, Option<f32>>(7)?, r.get::<_, Option<f32>>(8)?, r.get::<_, Option<f32>>(9)?) {
                    (Some(x), Some(y), Some(z), Some(yaw)) => Some([x, y, z, yaw]),
                    _ => None,
                };
                Ok(Account { id, username: r.get(0)?, admin: r.get::<_, i64>(1)? != 0, profile: Profile { name: r.get(2)?, race: r.get(3)?, gender: r.get(4)?, gear: r.get(5)? }, pos })
            })
            .optional()
            .ok()
            .flatten()
    }

    /// Where the character appears in the world: where it was last seen, else the capital of its race.
    pub fn spawn_of(&self, acc: &Account) -> [f32; 4] {
        acc.pos.unwrap_or_else(|| self.capital_of(&acc.profile.race))
    }

    pub fn capital_of(&self, race: &str) -> [f32; 4] {
        self.capitals.iter().find(|c| c.race == race).map_or([0.0, 512.0, 0.0, 0.0], |c| [c.x, c.y + 1.0, c.z, c.yaw])
    }

    pub fn save_pos(&self, id: i64, p: [f32; 4]) {
        if p.iter().all(|v| v.is_finite()) {
            let _ = self.conn.lock().unwrap().execute("UPDATE accounts SET x = ?1, y = ?2, z = ?3, yaw = ?4 WHERE id = ?5", params![p[0], p[1], p[2], p[3], id]);
        }
    }
}
