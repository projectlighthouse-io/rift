//! Editing rows and structure.

use futures::future::BoxFuture;
use types::{Error, RowEdit, SchemaChange, TableRef};

/// Applies row edits in one transaction.
///
/// Implemented by: PostgreSQL, MySQL, SQLite.
pub trait EditRows: Send + Sync {
    /// Applies all edits atomically; returns rows changed. Nothing is applied on error.
    fn apply<'a>(&'a self, edits: &'a [RowEdit]) -> BoxFuture<'a, Result<u64, Error>>;
}

/// Changes table structure.
///
/// Implemented by: PostgreSQL, MySQL, SQLite (limited), ClickHouse.
pub trait EditSchema: Send + Sync {
    /// DDL for the changes, without running it. Shown in the status bar before applying.
    fn preview(&self, table: &TableRef, changes: &[SchemaChange]) -> Result<Vec<String>, Error>;
    /// Runs the changes. Transactional where the engine allows it (not MySQL).
    fn apply_schema<'a>(
        &'a self,
        table: &'a TableRef,
        changes: &'a [SchemaChange],
    ) -> BoxFuture<'a, Result<(), Error>>;
    /// False when DDL cannot be rolled back (MySQL); the UI warns before applying.
    fn transactional_ddl(&self) -> bool {
        true
    }
}
