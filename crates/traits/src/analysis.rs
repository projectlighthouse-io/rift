//! Explain, aggregation and statistics.

use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;
use types::{
    AggregateRequest, ChartData, ColumnName, ColumnStatsReport, Error, ExplainPlan, MemoryReport,
    TableRef,
};

/// Shows query plans.
///
/// Implemented by: PostgreSQL, MySQL, SQLite, ClickHouse.
pub trait Explain: Send + Sync {
    /// Plan for a statement, without running it.
    fn explain<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<ExplainPlan, Error>>;
}

/// Runs chart `GROUP BY`s in the database.
///
/// Implemented by: PostgreSQL, MySQL, SQLite, ClickHouse.
pub trait Aggregate: Send + Sync {
    /// Aggregated series, capped at `request.limit` points.
    fn aggregate<'a>(
        &'a self,
        request: &'a AggregateRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChartData, Error>>;
}

/// Summarises a column with one `GROUP BY` over the table.
///
/// Implemented by: PostgreSQL, MySQL, SQLite, ClickHouse.
pub trait ColumnStats: Send + Sync {
    /// Rows, nulls, distinct and the `top` most common values.
    fn column_stats<'a>(
        &'a self,
        table: &'a TableRef,
        column: &'a ColumnName,
        top: u32,
    ) -> BoxFuture<'a, Result<ColumnStatsReport, Error>>;
}

/// Memory usage of a key-value store.
///
/// Implemented by: Redis (`INFO`, `MEMORY USAGE` on a sample), FoundationDB (status JSON).
pub trait MemoryStats: Send + Sync {
    /// Overview, possibly sampled.
    fn memory_report(&self) -> BoxFuture<'_, Result<MemoryReport, Error>>;
}
