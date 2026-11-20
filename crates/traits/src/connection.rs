//! The base connection trait.

use std::time::Duration;

use futures::future::BoxFuture;
use types::{ConnectionInfo, Error};

use crate::{
    Aggregate, Backup, Browse, ColumnStats, Describe, EditRows, EditSchema, Explain, KeyExpiry,
    KeyValue, MemoryStats, Query, TableTtl, Transactions,
};

/// An open connection. Implementations add capabilities by overriding the matching accessor to
/// return `Some(self)`; the UI shows a feature only when its accessor returns `Some`.
///
/// Connections are shared between views and background tasks, so they are handed out as
/// `Arc<dyn Connection>` (a documented exception to "avoid `Arc`").
pub trait Connection: Send + Sync {
    /// Facts about the server.
    fn info(&self) -> &ConnectionInfo;
    /// Round-trip to the server.
    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>>;
    /// Closes the connection and its pool.
    fn close(&self) -> BoxFuture<'_, Result<(), Error>>;

    /// Schemas, tables and views.
    fn browse(&self) -> Option<&dyn Browse> {
        None
    }
    /// Run SQL.
    fn query(&self) -> Option<&dyn Query> {
        None
    }
    /// Apply row edits.
    fn edit_rows(&self) -> Option<&dyn EditRows> {
        None
    }
    /// Change table structure.
    fn edit_schema(&self) -> Option<&dyn EditSchema> {
        None
    }
    /// Columns, indexes, foreign keys and DDL.
    fn describe(&self) -> Option<&dyn Describe> {
        None
    }
    /// Atomic batches.
    fn transactions(&self) -> Option<&dyn Transactions> {
        None
    }
    /// Key scans and reads/writes.
    fn key_value(&self) -> Option<&dyn KeyValue> {
        None
    }
    /// Per-key TTL.
    fn key_expiry(&self) -> Option<&dyn KeyExpiry> {
        None
    }
    /// Table and column TTL rules.
    fn table_ttl(&self) -> Option<&dyn TableTtl> {
        None
    }
    /// Query plans.
    fn explain(&self) -> Option<&dyn Explain> {
        None
    }
    /// Chart aggregation in the database.
    fn aggregate(&self) -> Option<&dyn Aggregate> {
        None
    }
    /// Column statistics.
    fn column_stats(&self) -> Option<&dyn ColumnStats> {
        None
    }
    /// Memory usage.
    fn memory_stats(&self) -> Option<&dyn MemoryStats> {
        None
    }
    /// Backup and restore.
    fn backup(&self) -> Option<&dyn Backup> {
        None
    }
}
