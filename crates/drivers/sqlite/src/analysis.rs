//! Aggregate and ColumnStats: one `GROUP BY` in the database, built by `dialect::chart`.

use dialect::{Dialect, chart};
use futures::FutureExt;
use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;
use traits::{Aggregate, ColumnStats};
use types::{AggregateRequest, ChartData, ColumnName, ColumnStatsReport, Error, TableRef, Value};

use crate::connection::SqliteConnection;
use crate::{error, values};

/// Every row of `sql`, decoded.
fn fetch_values(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<Vec<Value>>, Error> {
    let mut stmt = conn.prepare(sql).map_err(error::map)?;
    let n = stmt.column_count();
    let mut rows = stmt.query([]).map_err(error::map)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(error::map)? {
        out.push(
            (0..n)
                .map(|i| values::value(row.get_ref_unwrap(i), None))
                .collect(),
        );
    }
    Ok(out)
}

impl Aggregate for SqliteConnection {
    fn aggregate<'a>(
        &'a self,
        request: &'a AggregateRequest,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChartData, Error>> {
        async move {
            let sql = chart::aggregate_sql(Dialect::Sqlite, request)?;
            let rows = self.with(move |c| fetch_values(c, &sql)).await?;
            Ok(chart::chart_data(rows, request))
        }
        .boxed()
    }
}

impl ColumnStats for SqliteConnection {
    fn column_stats<'a>(
        &'a self,
        table: &'a TableRef,
        column: &'a ColumnName,
        top: u32,
    ) -> BoxFuture<'a, Result<ColumnStatsReport, Error>> {
        async move {
            let (totals, top) = chart::column_stats_sql(Dialect::Sqlite, table, column, top);
            let (totals, top) = self
                .with(move |c| Ok((fetch_values(c, &totals)?, fetch_values(c, &top)?)))
                .await?;
            Ok(chart::column_stats(
                &totals.into_iter().next().unwrap_or_default(),
                top,
            ))
        }
        .boxed()
    }
}
