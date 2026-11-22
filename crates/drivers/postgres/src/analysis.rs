//! Aggregate and ColumnStats: one `GROUP BY` in the database, built by `dialect::chart`.

use dialect::{Dialect, chart};
use futures::future::{self, BoxFuture, Either};
use futures::{FutureExt, TryStreamExt};
use sqlx::AssertSqlSafe;
use tokio_util::sync::CancellationToken;
use traits::{Aggregate, ColumnStats};
use types::{AggregateRequest, ChartData, ColumnName, ColumnStatsReport, Error, TableRef, Value};

use crate::connection::PgConnection;
use crate::error;
use crate::query::{decode_row, row_columns};

impl PgConnection {
    /// Every row of `sql`, decoded.
    pub(crate) async fn fetch_values(&self, sql: String) -> Result<Vec<Vec<Value>>, Error> {
        let rows: Vec<sqlx::postgres::PgRow> = sqlx::raw_sql(AssertSqlSafe(sql))
            .fetch(&self.pool)
            .try_collect()
            .await
            .map_err(error::map)?;
        let types: Vec<String> = rows.first().map_or_else(Vec::new, |r| {
            row_columns(r).into_iter().map(|c| c.type_name).collect()
        });
        Ok(rows.iter().map(|r| decode_row(r, &types)).collect())
    }
}

impl Aggregate for PgConnection {
    fn aggregate<'a>(
        &'a self,
        request: &'a AggregateRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChartData, Error>> {
        async move {
            let sql = chart::aggregate_sql(Dialect::Postgres, request)?;
            let rows = match future::select(
                Box::pin(self.fetch_values(sql)),
                Box::pin(cancel.cancelled()),
            )
            .await
            {
                Either::Left((rows, _)) => rows?,
                Either::Right(_) => return Err(Error::Cancelled),
            };
            Ok(chart::chart_data(rows, request))
        }
        .boxed()
    }
}

impl ColumnStats for PgConnection {
    fn column_stats<'a>(
        &'a self,
        table: &'a TableRef,
        column: &'a ColumnName,
        top: u32,
    ) -> BoxFuture<'a, Result<ColumnStatsReport, Error>> {
        async move {
            let (totals, top) = chart::column_stats_sql(Dialect::Postgres, table, column, top);
            let totals = self
                .fetch_values(totals)
                .await?
                .into_iter()
                .next()
                .unwrap_or_default();
            let top = self.fetch_values(top).await?;
            Ok(chart::column_stats(&totals, top))
        }
        .boxed()
    }
}
