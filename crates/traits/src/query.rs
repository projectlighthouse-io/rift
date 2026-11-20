//! Running SQL.

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;
use types::{Column, Error, Row};

/// Rows of a result, streamed in pages of up to [`types::PAGE_SIZE`]. Drivers feed it from a
/// bounded channel so a slow consumer applies backpressure.
pub struct RowStream {
    /// Result columns.
    pub columns: Vec<Column>,
    /// Pages of rows. Ends early with [`Error::Cancelled`] if the token fires.
    pub pages: BoxStream<'static, Result<Vec<Row>, Error>>,
}

/// Output of one statement.
#[non_exhaustive]
pub enum StatementOutput {
    /// A row-returning statement.
    Rows(RowStream),
    /// A statement that changed rows (or DDL with `rows == 0`).
    Affected {
        /// Rows changed.
        rows: u64,
    },
}

/// Runs a single SQL statement.
///
/// Implemented by: PostgreSQL, MySQL, SQLite, ClickHouse.
pub trait Query: Send + Sync {
    /// Runs one statement. Cancelling the token stops the server-side query when the driver
    /// can, and ends the stream with [`Error::Cancelled`].
    fn execute<'a>(
        &'a self,
        sql: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<StatementOutput, Error>>;
}
