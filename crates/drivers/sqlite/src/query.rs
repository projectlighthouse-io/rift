//! Running SQL: rows stream from a blocking thread through a bounded channel; cancelling
//! interrupts the statement.

use futures::channel::{mpsc, oneshot};
use futures::future::{self, BoxFuture};
use futures::{FutureExt, SinkExt, StreamExt};
use tokio_util::sync::CancellationToken;
use traits::{Explain, Query, RowStream, StatementOutput};
use types::{Column, Error, ExplainPlan, PAGE_SIZE, Row};

use crate::connection::{SqliteConnection, open};
use crate::{error, values};

/// Pages buffered between the database thread and the consumer.
const BUFFERED_PAGES: usize = 4;

enum Head {
    Rows(Vec<Column>),
    Affected(u64),
}

impl Query for SqliteConnection {
    fn execute<'a>(
        &'a self,
        sql: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<StatementOutput, Error>> {
        async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let (path, read_only, sql) = (self.path.clone(), self.read_only, sql.to_owned());
            let (head_tx, head_rx) = oneshot::channel::<Result<Head, Error>>();
            let (mut page_tx, page_rx) = mpsc::channel::<Result<Vec<Row>, Error>>(BUFFERED_PAGES);
            let (handle_tx, handle_rx) = oneshot::channel();
            let (done_tx, done_rx) = oneshot::channel::<()>();
            self.spawner.spawn_blocking(Box::new(move || {
                let _done = done_tx;
                let conn = match open(&path, read_only) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = head_tx.send(Err(e));
                        return;
                    }
                };
                let _ = handle_tx.send(conn.get_interrupt_handle());
                let mut stmt = match conn.prepare(&sql) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = head_tx.send(Err(error::map(e)));
                        return;
                    }
                };
                if stmt.column_count() == 0 {
                    let result = stmt
                        .raw_execute()
                        .map(|n| Head::Affected(n as u64))
                        .map_err(error::map);
                    let _ = head_tx.send(result);
                    return;
                }
                let decls: Vec<Option<String>> = stmt
                    .columns()
                    .iter()
                    .map(|c| c.decl_type().map(str::to_owned))
                    .collect();
                let columns = stmt
                    .column_names()
                    .iter()
                    .zip(&decls)
                    .map(|(n, d)| Column::new(*n, d.clone().unwrap_or_default().to_lowercase()))
                    .collect();
                let _ = head_tx.send(Ok(Head::Rows(columns)));
                let mut rows = match stmt.query([]) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = futures::executor::block_on(page_tx.send(Err(error::map(e))));
                        return;
                    }
                };
                let mut page = Vec::with_capacity(PAGE_SIZE);
                loop {
                    match rows.next() {
                        Ok(Some(row)) => {
                            page.push(Row {
                                values: (0..decls.len())
                                    .map(|i| {
                                        values::value(row.get_ref_unwrap(i), decls[i].as_deref())
                                    })
                                    .collect(),
                            });
                            if page.len() == PAGE_SIZE
                                && futures::executor::block_on(
                                    page_tx.send(Ok(std::mem::take(&mut page))),
                                )
                                .is_err()
                            {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = futures::executor::block_on(page_tx.send(Err(error::map(e))));
                            return;
                        }
                    }
                }
                if !page.is_empty() {
                    let _ = futures::executor::block_on(page_tx.send(Ok(page)));
                }
            }));
            // Interrupt the statement if the token fires before it finishes.
            if let Ok(handle) = handle_rx.await {
                self.spawner.spawn(
                    async move {
                        if let future::Either::Left(_) =
                            future::select(Box::pin(cancel.cancelled_owned()), done_rx).await
                        {
                            handle.interrupt();
                        }
                    }
                    .boxed(),
                );
            }
            match head_rx.await.map_err(|_| Error::Cancelled)?? {
                Head::Affected(rows) => Ok(StatementOutput::Affected { rows }),
                Head::Rows(columns) => Ok(StatementOutput::Rows(RowStream {
                    columns,
                    pages: page_rx.boxed(),
                })),
            }
        }
        .boxed()
    }
}

impl Explain for SqliteConnection {
    fn explain<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<ExplainPlan, Error>> {
        let sql = format!("EXPLAIN QUERY PLAN {sql}");
        async move {
            self.with(move |c| {
                let mut stmt = c.prepare(&sql).map_err(error::map)?;
                let lines: Vec<String> = stmt
                    .query_map([], |r| r.get::<_, String>(3))
                    .map_err(error::map)?
                    .filter_map(Result::ok)
                    .collect();
                Ok(ExplainPlan {
                    text: lines.join("\n"),
                })
            })
            .await
        }
        .boxed()
    }
}
