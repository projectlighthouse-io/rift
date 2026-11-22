//! Running SQL: streamed, cancellable, text protocol.

use futures::channel::{mpsc, oneshot};
use futures::future::{self, BoxFuture, Either};
use futures::{FutureExt, SinkExt, StreamExt, TryStreamExt};
use sqlx::{AssertSqlSafe, Column, PgPool, Row, TypeInfo, ValueRef};
use tokio_util::sync::CancellationToken;
use traits::{Explain, Query, RowStream, StatementOutput};
use types::{Error, ExplainPlan, PAGE_SIZE, Row as RiftRow, Value};

use crate::connection::PgConnection;
use crate::{error, values};

/// Pages buffered between the database task and the consumer.
const BUFFERED_PAGES: usize = 4;

pub(crate) fn type_name(info: &sqlx::postgres::PgTypeInfo) -> String {
    info.name().to_lowercase()
}

/// Names and types as the simple protocol reports them; custom types come back as `?`.
pub(crate) fn row_columns(row: &sqlx::postgres::PgRow) -> Vec<types::Column> {
    row.columns()
        .iter()
        .map(|c| types::Column::new(c.name(), type_name(c.type_info())))
        .collect()
}

/// Cancels the statement whose text starts with `tag`, from another pooled connection.
async fn cancel_tagged(pool: &PgPool, tag: &str) {
    let _ = sqlx::query(
        "SELECT pg_cancel_backend(pid) FROM pg_stat_activity \
         WHERE left(query, length($1)) = $1 AND pid <> pg_backend_pid()",
    )
    .bind(tag)
    .execute(pool)
    .await;
}

/// Sends `e` as the header if none went out yet, else as the next page.
async fn report(
    header_tx: &mut Option<oneshot::Sender<Header>>,
    tx: &mut mpsc::Sender<Result<Vec<RiftRow>, Error>>,
    e: Error,
) {
    match header_tx.take() {
        Some(h) => {
            let _ = h.send(Header::Failed(e));
        }
        None => {
            let _ = tx.send(Err(e)).await;
        }
    }
}

/// What the statement turned out to be, known once its first row or its end arrives.
enum Header {
    Rows(Vec<types::Column>),
    Done(u64),
    Failed(Error),
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
            // One round trip: the statement runs on the simple protocol while the describer reads
            // its column types in parallel. The tag lets cancel find the backend in
            // pg_stat_activity without first asking it for its pid.
            let tag = format!("/* rift:{} */", uuid::Uuid::new_v4());
            let tagged = format!("{tag} {sql}");
            let (described_tx, described_rx) = oneshot::channel();
            let describer = self.describer.clone();
            let sql = sql.to_owned();
            self.spawner.spawn(
                async move {
                    let _ = described_tx.send(describer.columns(sql).await);
                }
                .boxed(),
            );

            let (header_tx, header_rx) = oneshot::channel::<Header>();
            let (mut tx, rx) = mpsc::channel::<Result<Vec<RiftRow>, Error>>(BUFFERED_PAGES);
            let pool = self.pool.clone();
            self.spawner.spawn(
                async move {
                    let mut header_tx = Some(header_tx);
                    let mut conn = match pool.acquire().await {
                        Ok(c) => c,
                        Err(e) => {
                            report(&mut header_tx, &mut tx, error::map(e)).await;
                            return;
                        }
                    };
                    let mut stream = sqlx::raw_sql(AssertSqlSafe(tagged)).fetch_many(&mut *conn);
                    let mut affected = 0;
                    let mut type_names: Option<Vec<String>> = None;
                    let mut page = Vec::with_capacity(PAGE_SIZE);
                    loop {
                        let next =
                            future::select(stream.next(), Box::pin(cancel.cancelled())).await;
                        let item = match next {
                            Either::Left((item, _)) => item,
                            Either::Right(_) => {
                                drop(stream);
                                cancel_tagged(&pool, &tag).await;
                                report(&mut header_tx, &mut tx, Error::Cancelled).await;
                                return;
                            }
                        };
                        match item {
                            Some(Ok(sqlx::Either::Left(done))) => affected += done.rows_affected(),
                            Some(Ok(sqlx::Either::Right(row))) => {
                                let types = type_names.get_or_insert_with(|| {
                                    let columns = row_columns(&row);
                                    let names = columns.iter().map(|c| c.type_name.clone());
                                    let names = names.collect();
                                    if let Some(h) = header_tx.take() {
                                        let _ = h.send(Header::Rows(columns));
                                    }
                                    names
                                });
                                page.push(RiftRow {
                                    values: decode_row(&row, types),
                                });
                                if page.len() == PAGE_SIZE
                                    && tx.send(Ok(std::mem::take(&mut page))).await.is_err()
                                {
                                    return;
                                }
                            }
                            Some(Err(e)) => {
                                report(&mut header_tx, &mut tx, error::map(e)).await;
                                return;
                            }
                            None => break,
                        }
                    }
                    match header_tx.take() {
                        Some(h) => {
                            let _ = h.send(Header::Done(affected));
                        }
                        None if !page.is_empty() => {
                            let _ = tx.send(Ok(page)).await;
                        }
                        None => {}
                    }
                }
                .boxed(),
            );

            let described = || async { described_rx.await.ok().flatten() };
            let rows = |columns| {
                Ok(StatementOutput::Rows(RowStream {
                    columns,
                    pages: rx.boxed(),
                }))
            };
            match header_rx.await {
                // The simple protocol named every type: no need to wait for the describer.
                Ok(Header::Rows(columns)) if !columns.iter().any(|c| c.type_name == "?") => {
                    rows(columns)
                }
                Ok(Header::Rows(columns)) => rows(
                    described()
                        .await
                        .filter(|d| d.len() == columns.len())
                        .unwrap_or(columns),
                ),
                // No rows came back: a SELECT that matched nothing still has columns to show.
                Ok(Header::Done(n)) => match described().await {
                    Some(columns) if !columns.is_empty() => rows(columns),
                    _ => Ok(StatementOutput::Affected { rows: n }),
                },
                Ok(Header::Failed(e)) => Err(e),
                Err(_) => Err(Error::Cancelled),
            }
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
            let rows: Vec<sqlx::postgres::PgRow> =
                sqlx::raw_sql(AssertSqlSafe(format!("EXPLAIN {sql}")))
                    .fetch(&self.pool)
                    .try_collect()
                    .await
                    .map_err(error::map)?;
            let lines = rows
                .iter()
                .map(|r| r.try_get::<String, _>(0))
                .collect::<Result<Vec<_>, _>>()
                .map_err(error::map)?;
            Ok(ExplainPlan {
                text: lines.join("\n"),
            })
        }
        .boxed()
    }
}
