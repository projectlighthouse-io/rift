//! Rift app database (SQLite): saved connections, query history and dashboards.
//!
//! Records are stored as JSON bodies next to a few indexed columns, so adding a field to a
//! record type needs no migration. Passwords never touch this database.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use traits::{ConnectionStore, DashboardStore, HistoryStore};
use types::{ConnectionId, Dashboard, DashboardId, Error, HistoryEntry, SavedConnection};

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// The SQLite-backed implementation of every app store.
pub struct SqliteStore {
    db: Mutex<Connection>,
}

fn io(e: impl std::fmt::Display) -> Error {
    Error::io(e.to_string())
}

fn to_json<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(io)
}

fn from_json<T: DeserializeOwned>(body: &str) -> Result<T, Error> {
    serde_json::from_str(body).map_err(io)
}

impl SqliteStore {
    /// Opens (creating if needed) the database at `path` and runs migrations.
    ///
    /// # Errors
    /// [`Error::Io`] if the file cannot be opened or migrated.
    pub fn open(path: &Path) -> Result<Self, Error> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        Self::init(Connection::open(path).map_err(io)?)
    }

    /// An in-memory database, for tests.
    ///
    /// # Errors
    /// [`Error::Io`] if migrations fail.
    pub fn in_memory() -> Result<Self, Error> {
        Self::init(Connection::open_in_memory().map_err(io)?)
    }

    fn init(mut db: Connection) -> Result<Self, Error> {
        db.pragma_update(None, "journal_mode", "WAL").map_err(io)?;
        embedded::migrations::runner().run(&mut db).map_err(io)?;
        Ok(Self { db: Mutex::new(db) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().expect("store mutex poisoned")
    }

    fn bodies<T: DeserializeOwned>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<T>, Error> {
        let db = self.db();
        let mut stmt = db.prepare(sql).map_err(io)?;
        let rows = stmt
            .query_map(params, |row| row.get::<_, String>(0))
            .map_err(io)?;
        rows.map(|body| from_json(&body.map_err(io)?)).collect()
    }
}

impl ConnectionStore for SqliteStore {
    fn list(&self) -> Result<Vec<SavedConnection>, Error> {
        self.bodies(
            "SELECT body FROM connections ORDER BY grp IS NOT NULL, grp, name COLLATE NOCASE",
            [],
        )
    }

    fn save(&self, connection: &SavedConnection) -> Result<(), Error> {
        self.db()
            .execute(
                "INSERT INTO connections (id, name, driver, grp, body) VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT (id) DO UPDATE SET name = ?2, driver = ?3, grp = ?4, body = ?5",
                params![
                    connection.id.to_string(),
                    connection.name,
                    connection.driver.as_str(),
                    connection.group,
                    to_json(connection)?
                ],
            )
            .map_err(io)?;
        Ok(())
    }

    fn delete(&self, id: ConnectionId) -> Result<(), Error> {
        let mut db = self.db();
        let tx = db.transaction().map_err(io)?;
        let id = id.to_string();
        tx.execute("DELETE FROM connections WHERE id = ?1", [&id])
            .map_err(io)?;
        tx.execute("DELETE FROM history WHERE connection = ?1", [&id])
            .map_err(io)?;
        tx.execute("DELETE FROM dashboards WHERE connection = ?1", [&id])
            .map_err(io)?;
        tx.commit().map_err(io)
    }
}

impl HistoryStore for SqliteStore {
    fn append(&self, entry: &HistoryEntry) -> Result<(), Error> {
        self.db()
            .execute(
                "INSERT INTO history (connection, ran_at, body) VALUES (?1, ?2, ?3)",
                params![
                    entry.connection.to_string(),
                    entry.ran_at.to_rfc3339(),
                    to_json(entry)?
                ],
            )
            .map_err(io)?;
        Ok(())
    }

    fn recent(
        &self,
        connection: Option<ConnectionId>,
        limit: u32,
    ) -> Result<Vec<HistoryEntry>, Error> {
        match connection {
            Some(c) => self.bodies(
                "SELECT body FROM history WHERE connection = ?1 ORDER BY id DESC LIMIT ?2",
                params![c.to_string(), limit],
            ),
            None => self.bodies(
                "SELECT body FROM history ORDER BY id DESC LIMIT ?1",
                [limit],
            ),
        }
    }
}

impl DashboardStore for SqliteStore {
    fn list(&self, connection: ConnectionId) -> Result<Vec<Dashboard>, Error> {
        self.bodies(
            "SELECT body FROM dashboards WHERE connection = ?1 ORDER BY rowid",
            [connection.to_string()],
        )
    }

    fn save(&self, dashboard: &Dashboard) -> Result<(), Error> {
        self.db()
            .execute(
                "INSERT INTO dashboards (id, connection, body) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (id) DO UPDATE SET connection = ?2, body = ?3",
                params![
                    dashboard.id.to_string(),
                    dashboard.connection.to_string(),
                    to_json(dashboard)?
                ],
            )
            .map_err(io)?;
        Ok(())
    }

    fn delete(&self, id: DashboardId) -> Result<(), Error> {
        self.db()
            .execute("DELETE FROM dashboards WHERE id = ?1", [id.to_string()])
            .map_err(io)?;
        Ok(())
    }
}
