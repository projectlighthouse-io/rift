//! Atomic batches.

use futures::future::BoxFuture;
use types::{Error, TxOp};

/// Runs a batch of writes atomically: all apply or none do.
///
/// Implemented by: PostgreSQL, MySQL, SQLite (SQL ops), Redis (`MULTI`/`EXEC`), FoundationDB.
pub trait Transactions: Send + Sync {
    /// Runs the batch. Ops the engine does not understand return [`Error::Unsupported`]
    /// before anything is written.
    fn atomic<'a>(&'a self, ops: &'a [TxOp]) -> BoxFuture<'a, Result<(), Error>>;
}
