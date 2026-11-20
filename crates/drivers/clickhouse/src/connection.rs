//! The ClickHouse connection: Query, Explain, Aggregate, ColumnStats, EditSchema, TableTtl.

use std::time::{Duration, Instant};

use dialect::{Dialect, chart, ddl};
use futures::channel::mpsc;
use futures::future::{self, BoxFuture, Either};
use futures::{FutureExt, SinkExt, StreamExt};
use tokio_util::sync::CancellationToken;
use traits::{
    Aggregate, Browse, ColumnStats, Connection, Describe, EditSchema, Explain, Query, RowStream,
    Spawner, StatementOutput, TableTtl,
};
use types::{
    AggregateRequest, ChartData, Column, ColumnName, ColumnStatsReport, ConnectionInfo, Error,
    ExplainPlan, PAGE_SIZE, Row, SchemaChange, TableRef, TtlRule,
};

use crate::client::Client;

const BUFFERED_PAGES: usize = 4;

/// A ClickHouse server reached over HTTP.
pub(crate) struct ClickHouseConnection {
    pub(crate) client: Client,
    pub(crate) spawner: std::sync::Arc<dyn Spawner>,
    pub(crate) info: ConnectionInfo,
}

impl Connection for ClickHouseConnection {
    fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        async move {
            let started = Instant::now();
            self.client.values("SELECT 1").await?;
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

    fn aggregate(&self) -> Option<&dyn Aggregate> {
        Some(self)
    }

    fn column_stats(&self) -> Option<&dyn ColumnStats> {
        Some(self)
    }

    fn edit_schema(&self) -> Option<&dyn EditSchema> {
        Some(self)
    }

    fn table_ttl(&self) -> Option<&dyn TableTtl> {
        Some(self)
    }
}

impl Query for ClickHouseConnection {
    fn execute<'a>(
        &'a self,
        sql: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<StatementOutput, Error>> {
        async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let id = uuid::Uuid::new_v4().to_string();
            let started = self.client.run(sql, Some(&id));
            let response = match future::select(
                Box::pin(started),
                Box::pin(cancel.clone().cancelled_owned()),
            )
            .await
            {
                Either::Left((r, _)) => r?,
                Either::Right(_) => {
                    self.client.kill(&id).await;
                    return Err(Error::Cancelled);
                }
            };
            if response.names.is_empty() {
                return Ok(StatementOutput::Affected {
                    rows: response.written_rows,
                });
            }
            let columns: Vec<Column> = response
                .names
                .iter()
                .zip(&response.types)
                .map(|(n, t)| Column::new(n.as_str(), t.as_str()))
                .collect();
            let (mut tx, rx) = mpsc::channel::<Result<Vec<Row>, Error>>(BUFFERED_PAGES);
            let client = self.client.clone();
            let mut rows = response.rows;
            self.spawner.spawn(
                async move {
                    let mut page = Vec::with_capacity(PAGE_SIZE);
                    loop {
                        match future::select(rows.next(), Box::pin(cancel.cancelled())).await {
                            Either::Right(_) => {
                                drop(rows);
                                client.kill(&id).await;
                                let _ = tx.send(Err(Error::Cancelled)).await;
                                return;
                            }
                            Either::Left((None, _)) => break,
                            Either::Left((Some(Err(e)), _)) => {
                                let _ = tx.send(Err(e)).await;
                                return;
                            }
                            Either::Left((Some(Ok(values)), _)) => {
                                page.push(Row { values });
                                if page.len() == PAGE_SIZE
                                    && tx.send(Ok(std::mem::take(&mut page))).await.is_err()
                                {
                                    client.kill(&id).await;
                                    return;
                                }
                            }
                        }
                    }
                    if !page.is_empty() {
                        let _ = tx.send(Ok(page)).await;
                    }
                }
                .boxed(),
            );
            Ok(StatementOutput::Rows(RowStream {
                columns,
                pages: rx.boxed(),
            }))
        }
        .boxed()
    }
}

impl Explain for ClickHouseConnection {
    fn explain<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<ExplainPlan, Error>> {
        async move {
            let rows = self.client.values(&format!("EXPLAIN {sql}")).await?;
            let lines: Vec<String> = rows
                .into_iter()
                .filter_map(|r| r.into_iter().next())
                .map(|v| v.to_string())
                .collect();
            Ok(ExplainPlan {
                text: lines.join("\n"),
            })
        }
        .boxed()
    }
}

impl Aggregate for ClickHouseConnection {
    fn aggregate<'a>(
        &'a self,
        request: &'a AggregateRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChartData, Error>> {
        async move {
            let sql = chart::aggregate_sql(Dialect::ClickHouse, request)?;
            let rows = match future::select(
                Box::pin(self.client.values(&sql)),
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

impl ColumnStats for ClickHouseConnection {
    fn column_stats<'a>(
        &'a self,
        table: &'a TableRef,
        column: &'a ColumnName,
        top: u32,
    ) -> BoxFuture<'a, Result<ColumnStatsReport, Error>> {
        async move {
            let (totals, top) = chart::column_stats_sql(Dialect::ClickHouse, table, column, top);
            let totals = self
                .client
                .values(&totals)
                .await?
                .into_iter()
                .next()
                .unwrap_or_default();
            let top = self.client.values(&top).await?;
            Ok(chart::column_stats(&totals, top))
        }
        .boxed()
    }
}

impl EditSchema for ClickHouseConnection {
    fn preview(&self, table: &TableRef, changes: &[SchemaChange]) -> Result<Vec<String>, Error> {
        ddl::schema_changes_sql(Dialect::ClickHouse, table, changes)
    }

    fn apply_schema<'a>(
        &'a self,
        table: &'a TableRef,
        changes: &'a [SchemaChange],
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            for sql in self.preview(table, changes)? {
                self.client.values(&sql).await?;
            }
            Ok(())
        }
        .boxed()
    }

    fn transactional_ddl(&self) -> bool {
        false
    }
}

/// The `TTL …` clause of a `CREATE TABLE` statement, without the keyword.
pub(crate) fn table_ttl_clause(create: &str) -> Option<String> {
    let start = create
        .find("\nTTL ")
        .map(|i| i + 5)
        .or_else(|| create.find(" TTL ").map(|i| i + 5))?;
    let rest = &create[start..];
    let end = rest
        .find("\nSETTINGS")
        .or_else(|| rest.find(" SETTINGS"))
        .unwrap_or(rest.len());
    let clause = rest[..end].trim();
    (!clause.is_empty()).then(|| clause.to_owned())
}

impl TableTtl for ClickHouseConnection {
    fn ttl_rules<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<Vec<TtlRule>, Error>> {
        async move {
            let d = Dialect::ClickHouse;
            let sql = format!(
                "SELECT create_table_query FROM system.tables WHERE database = {} AND name = {}",
                d.string(table.schema.as_ref().map_or("default", |s| s.as_str())),
                d.string(table.name.as_str())
            );
            let rows = self.client.values(&sql).await?;
            let create = rows
                .into_iter()
                .next()
                .and_then(|r| r.into_iter().next())
                .ok_or_else(|| Error::not_found(table.to_string()))?;
            Ok(table_ttl_clause(&create.to_string())
                .map(|expression| TtlRule {
                    column: None,
                    expression,
                })
                .into_iter()
                .collect())
        }
        .boxed()
    }

    fn set_table_ttl<'a>(
        &'a self,
        table: &'a TableRef,
        rule: Option<&'a TtlRule>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let t = Dialect::ClickHouse.table(table);
            let sql = match rule {
                Some(r) => format!("ALTER TABLE {t} MODIFY TTL {}", r.expression),
                None => format!("ALTER TABLE {t} REMOVE TTL"),
            };
            self.client.values(&sql).await.map(|_| ())
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_ttl_clause() {
        let create = "CREATE TABLE analytics.events (`ts` DateTime) ENGINE = MergeTree PARTITION BY toDate(ts) ORDER BY ts TTL ts + toIntervalDay(90) SETTINGS index_granularity = 8192";
        assert_eq!(
            super::table_ttl_clause(create).as_deref(),
            Some("ts + toIntervalDay(90)")
        );
        assert_eq!(
            super::table_ttl_clause("CREATE TABLE t (x UInt8) ENGINE = Log"),
            None
        );
    }
}
