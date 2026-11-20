//! Running SQL: streamed, cancellable, text protocol.

use futures::channel::mpsc;
use futures::future::{self, BoxFuture, Either};
use futures::{FutureExt, SinkExt, StreamExt};
use sqlx::{AssertSqlSafe, Column, Executor, Row, SqlSafeStr, Statement, TypeInfo, ValueRef};
use tokio_util::sync::CancellationToken;
use traits::{Explain, Query, RowStream, StatementOutput};
use types::{Error, ExplainPlan, PAGE_SIZE, Row as RiftRow, Value};

use crate::connection::PgConnection;
use crate::{error, values};

/// Pages buffered between the database task and the consumer.
const BUFFERED_PAGES: usize = 4;

fn type_name(info: &sqlx::postgres::PgTypeInfo) -> String {
    info.name().to_lowercase()
}

impl PgConnection {
    /// Asks the server to cancel whatever backend `pid` is running, over a side connection.
    async fn cancel_backend(&self, pid: i32) {
        use sqlx::ConnectOptions;
        if let Ok(mut side) = self.options.connect().await {
            let _ = sqlx::query("SELECT pg_cancel_backend($1)")
                .bind(pid)
                .execute(&mut side)
                .await;
        }
    }
}

impl Query for PgConnection {
    fn execute<'a>(
        &'a self,
        sql: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<StatementOutput, Error>> {
        async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let mut conn = self.pool.acquire().await.map_err(error::map)?;
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *conn)
                .await
                .map_err(error::map)?;
            let described = (&mut *conn)
                .prepare(AssertSqlSafe(sql.to_owned()).into_sql_str())
                .await
                .map_err(error::map)?;
            let columns: Vec<types::Column> = described
                .columns()
                .iter()
                .map(|c| types::Column::new(c.name(), type_name(c.type_info())))
                .collect();
            let type_names: Vec<String> = columns.iter().map(|c| c.type_name.clone()).collect();
            let sql = sql.to_owned();

            if columns.is_empty() {
                let run = sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut *conn);
                return match future::select(Box::pin(run), Box::pin(cancel.cancelled())).await {
                    Either::Left((result, _)) => Ok(StatementOutput::Affected {
                        rows: result.map_err(error::map)?.rows_affected(),
                    }),
                    Either::Right(_) => {
                        self.cancel_backend(pid).await;
                        Err(Error::Cancelled)
                    }
                };
            }

            let (mut tx, rx) = mpsc::channel::<Result<Vec<RiftRow>, Error>>(BUFFERED_PAGES);
            let side = PgConnection {
                pool: self.pool.clone(),
                options: self.options.clone(),
                spawner: self.spawner.clone(),
                info: self.info.clone(),
            };
            self.spawner.spawn(
                async move {
                    let mut rows = sqlx::raw_sql(AssertSqlSafe(sql)).fetch(&mut *conn);
                    let mut page = Vec::with_capacity(PAGE_SIZE);
                    loop {
                        let next = future::select(rows.next(), Box::pin(cancel.cancelled())).await;
                        let item = match next {
                            Either::Left((item, _)) => item,
                            Either::Right(_) => {
                                drop(rows);
                                side.cancel_backend(pid).await;
                                let _ = tx.send(Err(Error::Cancelled)).await;
                                return;
                            }
                        };
                        match item {
                            Some(Ok(row)) => {
                                page.push(RiftRow {
                                    values: decode_row(&row, &type_names),
                                });
                                if page.len() == PAGE_SIZE
                                    && tx.send(Ok(std::mem::take(&mut page))).await.is_err()
                                {
                                    return;
                                }
                            }
                            Some(Err(e)) => {
                                let _ = tx.send(Err(error::map(e))).await;
                                return;
                            }
                            None => break,
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

/// Decodes a text-protocol row using the described column types.
pub(crate) fn decode_row(row: &sqlx::postgres::PgRow, type_names: &[String]) -> Vec<Value> {
    (0..row.len())
        .map(|i| {
            let fallback = || type_name(row.columns()[i].type_info());
            match row.try_get_raw(i) {
                Ok(raw) if raw.is_null() => Value::Null,
                Ok(raw) => match raw.as_str() {
                    Ok(text) => {
                        let name = type_names.get(i).cloned().unwrap_or_else(fallback);
                        values::parse(&name, text)
                    }
                    Err(_) => Value::Bytes(raw.as_bytes().unwrap_or_default().to_vec()),
                },
                Err(e) => Value::Unknown {
                    type_name: fallback(),
                    raw: e.to_string(),
                },
            }
        })
        .collect()
}

impl Explain for PgConnection {
    fn explain<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<ExplainPlan, Error>> {
        async move {
            let lines: Vec<String> = sqlx::query_scalar(AssertSqlSafe(format!("EXPLAIN {sql}")))
                .fetch_all(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(ExplainPlan {
                text: lines.join("\n"),
            })
        }
        .boxed()
    }
}
