//! SQLite persistence of the living NPCs: identity, skills, home, needs, goal and last position. The whole population is written in one
//! transaction every few seconds and on shutdown; the place count is stored as a guard, so a save made against a different place list is ignored.
use rusqlite::{params, Connection};

use crate::npcs::NpcState;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        // an older save has a `level` column instead of `skills`: those NPCs are regenerated, their levels mean nothing in a skill-based world
        let old: bool = conn.prepare("SELECT 1 FROM pragma_table_info('npcs') WHERE name = 'level'")?.exists([])?;
        if old {
            conn.execute_batch("DROP TABLE npcs; DELETE FROM meta;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS npcs (
                idx INTEGER PRIMARY KEY, name TEXT NOT NULL, race TEXT NOT NULL, gender TEXT NOT NULL, gear TEXT NOT NULL,
                skills TEXT NOT NULL DEFAULT '{}', home INTEGER NOT NULL, at_place INTEGER NOT NULL,
                goal_kind INTEGER NOT NULL, goal_a REAL NOT NULL, goal_dest INTEGER NOT NULL,
                fatigue REAL NOT NULL, x REAL NOT NULL, z REAL NOT NULL, yaw REAL NOT NULL, rng INTEGER NOT NULL);",
        )?;
        // saves from before NPCs carried weapons: add the column (it already exists in newer files, which is the error we ignore)
        let _ = conn.execute("ALTER TABLE npcs ADD COLUMN weapon TEXT NOT NULL DEFAULT ''", []);
        // ... and before NPCs traded (role, gold, goods)
        let _ = conn.execute("ALTER TABLE npcs ADD COLUMN role INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE npcs ADD COLUMN gold REAL NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE npcs ADD COLUMN stock TEXT NOT NULL DEFAULT '{}'", []);
        Ok(Store { conn })
    }

    /// The saved NPCs, or none when they were saved against a different number of places or vendors (the vendors come first among the ids).
    pub fn load(&self, places: usize, vendors: usize) -> rusqlite::Result<Vec<NpcState>> {
        let meta = |k: &str| -> Option<i64> { self.conn.query_row("SELECT v FROM meta WHERE k = ?1", [k], |r| r.get(0)).ok() };
        if meta("places") != Some(places as i64) || meta("vendors") != Some(vendors as i64) {
            return Ok(Vec::new());
        }
        let mut q = self.conn.prepare("SELECT idx, name, race, gender, gear, skills, home, at_place, goal_kind, goal_a, goal_dest, fatigue, x, z, yaw, rng, weapon, role, gold, stock FROM npcs ORDER BY idx")?;
        let rows = q.query_map([], |r| {
            Ok(NpcState {
                idx: r.get(0)?,
                name: r.get(1)?,
                race: r.get(2)?,
                gender: r.get(3)?,
                gear: r.get(4)?,
                skills: r.get(5)?,
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
                weapon: r.get(16)?,
                role: r.get(17)?,
                gold: r.get(18)?,
                stock: r.get(19)?,
            })
        })?;
        rows.collect()
    }

    pub fn save(&mut self, places: usize, vendors: usize, states: &[NpcState]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT OR REPLACE INTO meta (k, v) VALUES ('places', ?1)", params![places as i64])?;
        tx.execute("INSERT OR REPLACE INTO meta (k, v) VALUES ('vendors', ?1)", params![vendors as i64])?;
        tx.execute("DELETE FROM npcs", [])?;
        {
            let mut ins = tx.prepare("INSERT INTO npcs (idx, name, race, gender, gear, skills, home, at_place, goal_kind, goal_a, goal_dest, fatigue, x, z, yaw, rng, weapon, role, gold, stock) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)")?;
            for s in states {
                ins.execute(params![s.idx, s.name, s.race, s.gender, s.gear, s.skills, s.home as i64, s.at as i64, s.goal_kind, s.goal_a, s.goal_dest, s.fatigue, s.x, s.z, s.yaw, s.rng, s.weapon, s.role, s.gold, s.stock])?;
            }
        }
        tx.commit()
    }
}
