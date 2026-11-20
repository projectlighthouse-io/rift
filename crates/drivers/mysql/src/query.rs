//! Running SQL: streamed, cancellable, text protocol.

use futures::channel::mpsc;
use futures::future::{self, BoxFuture, Either};
use futures::{FutureExt, SinkExt, StreamExt};
use sqlx::{
    AssertSqlSafe, Column, ConnectOptions, Executor, Row, SqlSafeStr, Statement, TypeInfo, ValueRef,
};
use tokio_util::sync::CancellationToken;
use traits::{Explain, Query, RowStream, StatementOutput};
use types::{Error, ExplainPlan, PAGE_SIZE, Row as RiftRow, Value};

use crate::connection::MySqlConnection;
use crate::{error, values};

/// Pages buffered between the database task and the consumer.
const BUFFERED_PAGES: usize = 4;

/// Asks the server to stop the statement running on connection `id`, over a side connection.
async fn kill_query(options: sqlx::mysql::MySqlConnectOptions, id: u64) {
    if let Ok(mut side) = options.connect().await {
        let _ = sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {id}")))
            .execute(&mut side)
            .await;
    }
}

/// Decodes a text-protocol row using the prepared column types.
pub(crate) fn decode_row(row: &sqlx::mysql::MySqlRow, type_names: &[String]) -> Vec<Value> {
    (0..row.len())
        .map(|i| match row.try_get_raw(i) {
            Ok(raw) if raw.is_null() => Value::Null,
            Ok(raw) => {
                let name = type_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| row.columns()[i].type_info().name().to_owned());
                let bytes: &[u8] = sqlx::Decode::<sqlx::MySql>::decode(raw).unwrap_or_default();
                match std::str::from_utf8(bytes) {
                    Ok(text) => values::parse(&name, text),
                    Err(_) => Value::Bytes(bytes.to_vec()),
                }
            }
            Err(e) => Value::Unknown {
                type_name: String::new(),
                raw: e.to_string(),
            },
        })
        .collect()
}

impl Query for MySqlConnection {
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
            let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *conn)
                .await
                .map_err(error::map)?;
            // Some statements cannot be prepared (e.g. some SHOW forms); run them as plain
            // statements and read columns from the first row.
            let prepared = (&mut *conn)
                .prepare(AssertSqlSafe(sql.to_owned()).into_sql_str())
                .await;
            let (columns, type_names) = match &prepared {
                Ok(stmt) => {
                    let type_names: Vec<String> = stmt
                        .columns()
                        .iter()
                        .map(|c| c.type_info().name().to_owned())
                        .collect();
                    let columns = stmt
                        .columns()
                        .iter()
                        .zip(&type_names)
                        .map(|(c, t)| types::Column::new(c.name(), t.to_lowercase()))
                        .collect::<Vec<_>>();
                    (columns, type_names)
                }
                Err(_) => (Vec::new(), Vec::new()),
            };
            if let Err(e) = prepared
                && !matches!(&e, sqlx::Error::Database(db) if db.code().as_deref() == Some("1295"))
            {
                return Err(error::map(e));
            }
            let sql = sql.to_owned();
            if columns.is_empty() {
                let run = sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut *conn);
                return match future::select(Box::pin(run), Box::pin(cancel.cancelled())).await {
                    Either::Left((result, _)) => Ok(StatementOutput::Affected {
                        rows: result.map_err(error::map)?.rows_affected(),
                    }),
                    Either::Right(_) => {
                        kill_query(self.options.clone(), id).await;
                        Err(Error::Cancelled)
                    }
                };
            }
            let (mut tx, rx) = mpsc::channel::<Result<Vec<RiftRow>, Error>>(BUFFERED_PAGES);
            let options = self.options.clone();
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
                                kill_query(options, id).await;
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

impl Explain for MySqlConnection {
    fn explain<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<ExplainPlan, Error>> {
        async move {
            let lines: Vec<String> =
                sqlx::query_scalar(AssertSqlSafe(format!("EXPLAIN FORMAT=TREE {sql}")))
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
