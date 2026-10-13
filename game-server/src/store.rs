//! SQLite persistence of the living NPCs: identity, level, home, needs, goal and last position. The whole population is written in one
//! transaction every few seconds and on shutdown; the place count is stored as a guard, so a save made against a different place list is ignored.
use rusqlite::{params, Connection};

use crate::npcs::NpcState;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS npcs (
                idx INTEGER PRIMARY KEY, name TEXT NOT NULL, race TEXT NOT NULL, gender TEXT NOT NULL, gear TEXT NOT NULL,
                level INTEGER NOT NULL, home INTEGER NOT NULL, at_place INTEGER NOT NULL,
                goal_kind INTEGER NOT NULL, goal_a REAL NOT NULL, goal_dest INTEGER NOT NULL,
                fatigue REAL NOT NULL, x REAL NOT NULL, z REAL NOT NULL, yaw REAL NOT NULL, rng INTEGER NOT NULL);",
        )?;
        Ok(Store { conn })
    }

    /// The saved NPCs, or none when they were saved against a different number of places.
    pub fn load(&self, places: usize) -> rusqlite::Result<Vec<NpcState>> {
        let saved: Option<i64> = self.conn.query_row("SELECT v FROM meta WHERE k = 'places'", [], |r| r.get(0)).ok();
        if saved != Some(places as i64) {
            return Ok(Vec::new());
        }
        let mut q = self.conn.prepare("SELECT idx, name, race, gender, gear, level, home, at_place, goal_kind, goal_a, goal_dest, fatigue, x, z, yaw, rng FROM npcs ORDER BY idx")?;
        let rows = q.query_map([], |r| {
            Ok(NpcState {
                idx: r.get(0)?,
                name: r.get(1)?,
                race: r.get(2)?,
                gender: r.get(3)?,
                gear: r.get(4)?,
                level: r.get(5)?,
                home: r.get::<_, i64>(6)? as usize,
                at: r.get::<_, i64>(7)? as usize,
                goal_kind: r.get(8)?,
                goal_a: r.get(9)?,
                goal_dest: r.get(10)?,
                fatigue: r.get(11)?,
                x: r.get(12)?,
                z: r.get(13)?,
                yaw: r.get(14)?,
                rng: r.get(15)?,
            })
        })?;
        rows.collect()
    }

    pub fn save(&mut self, places: usize, states: &[NpcState]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT OR REPLACE INTO meta (k, v) VALUES ('places', ?1)", params![places as i64])?;
        tx.execute("DELETE FROM npcs", [])?;
        {
            let mut ins = tx.prepare("INSERT INTO npcs VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)")?;
            for s in states {
                ins.execute(params![s.idx, s.name, s.race, s.gender, s.gear, s.level, s.home as i64, s.at as i64, s.goal_kind, s.goal_a, s.goal_dest, s.fatigue, s.x, s.z, s.yaw, s.rng])?;
            }
        }
        tx.commit()
    }
}
