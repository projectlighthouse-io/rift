//! Backup and restore.

use std::path::Path;

use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;
use types::{BackupReport, Error};

/// Writes and restores backups, usually by running the engine's own tool.
///
/// Implemented by: PostgreSQL (`pg_dump`), MySQL (`mysqldump`), SQLite (file copy),
/// ClickHouse (`BACKUP`), Redis (`BGSAVE`), FoundationDB (`fdbbackup`).
pub trait Backup: Send + Sync {
    /// Writes a backup to `dest`. Fails with [`Error::MissingDependency`] if the tool is missing.
    fn backup<'a>(
        &'a self,
        dest: &'a Path,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<BackupReport, Error>>;
    /// Restores from `src`.
    fn restore<'a>(
        &'a self,
        src: &'a Path,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<(), Error>>;
}
