//! Backup and restore with SQLite's online backup API: no external tool needed.

use std::path::Path;

use futures::FutureExt;
use futures::future::BoxFuture;
use rusqlite::MAIN_DB;
use tokio_util::sync::CancellationToken;
use traits::Backup;
use types::{BackupReport, Error};

use crate::connection::SqliteConnection;
use crate::error;

impl Backup for SqliteConnection {
    fn backup<'a>(
        &'a self,
        dest: &'a Path,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<BackupReport, Error>> {
        async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let target = dest.to_path_buf();
            self.with(move |c| c.backup(MAIN_DB, &target, None).map_err(error::map))
                .await?;
            let bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
            Ok(BackupReport {
                path: dest.to_path_buf(),
                bytes,
            })
        }
        .boxed()
    }

    fn restore<'a>(
        &'a self,
        src: &'a Path,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let source = src.to_path_buf();
            self.with(move |c| {
                c.restore(MAIN_DB, &source, None::<fn(rusqlite::backup::Progress)>)
                    .map_err(error::map)
            })
            .await
        }
        .boxed()
    }
}
