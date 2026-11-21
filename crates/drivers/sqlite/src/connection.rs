//! The SQLite connection and its capability accessors.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::BoxFuture;
use rusqlite::OpenFlags;
use traits::{
    Aggregate, Backup, Browse, ColumnStats, Connection, Describe, EditRows, EditSchema, Explain,
    Query, Spawner, Transactions,
};
use types::{ConnectionInfo, Error};

use crate::error;

/// A SQLite database file.
pub(crate) struct SqliteConnection {
    pub(crate) path: PathBuf,
    pub(crate) read_only: bool,
    pub(crate) spawner: Arc<dyn Spawner>,
    pub(crate) info: ConnectionInfo,
}

impl SqliteConnection {
    /// Runs `f` with a fresh connection on a blocking thread.
    pub(crate) async fn with<T, F>(&self, f: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(&mut rusqlite::Connection) -> Result<T, Error> + Send + 'static,
    {
        let (path, read_only) = (self.path.clone(), self.read_only);
        traits::spawner::run_blocking(&*self.spawner, move || {
            let mut conn = open(&path, read_only)?;
            f(&mut conn)
        })
        .await?
    }
}

pub(crate) fn open(path: &std::path::Path, read_only: bool) -> Result<rusqlite::Connection, Error> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
    };
    let conn = rusqlite::Connection::open_with_flags(path, flags)
        .map_err(|e| Error::connect(e.to_string()))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(error::map)?;
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(error::map)?;
    Ok(conn)
}

impl Connection for SqliteConnection {
    fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        async move {
            let started = Instant::now();
            self.with(|c| c.query_row("SELECT 1", [], |_| Ok(())).map_err(error::map))
                .await?;
            Ok(started.elapsed())
        }
        .boxed()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), Error>> {
        async { Ok(()) }.boxed()
    }

    fn browse(&self) -> Option<&dyn Browse> {
        Some(self)
    }

    fn query(&self) -> Option<&dyn Query> {
        Some(self)
    }

    fn describe(&self) -> Option<&dyn Describe> {
        Some(self)
    }

    fn explain(&self) -> Option<&dyn Explain> {
        Some(self)
    }

    fn backup(&self) -> Option<&dyn Backup> {
        Some(self)
    }

    fn aggregate(&self) -> Option<&dyn Aggregate> {
        Some(self)
    }

    fn column_stats(&self) -> Option<&dyn ColumnStats> {
        Some(self)
    }

    fn edit_rows(&self) -> Option<&dyn EditRows> {
        Some(self)
    }

    fn edit_schema(&self) -> Option<&dyn EditSchema> {
        Some(self)
    }

    fn transactions(&self) -> Option<&dyn Transactions> {
        Some(self)
    }
}
