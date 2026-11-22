//! The Postgres connection and its capability accessors.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::BoxFuture;
use futures::lock::Mutex;
use sqlx::postgres::PgConnectOptions;
use sqlx::{
    AssertSqlSafe, Column, ConnectOptions, Executor, PgPool, SqlSafeStr, Statement, TypeInfo,
};
use traits::{
    Aggregate, Backup, Browse, ColumnStats, Connection, Describe, EditRows, EditSchema, Explain,
    Query, Spawner, Transactions,
};
use types::{ConnectionInfo, Error};

use crate::error;

/// A pooled Postgres connection.
pub(crate) struct PgConnection {
    pub(crate) pool: PgPool,
    /// Reads result column types for user SQL.
    pub(crate) describer: Describer,
    /// For `pg_dump` and `psql`.
    pub(crate) options: PgConnectOptions,
    pub(crate) spawner: Arc<dyn Spawner>,
    pub(crate) info: ConnectionInfo,
    /// For `pg_dump` and `psql`.
    pub(crate) password: Option<String>,
}

/// A side connection that only describes statements. It has no statement cache, so a table
/// changed by `ALTER TABLE` is never described from stale metadata; the pool keeps its cache for
/// the fixed catalog queries.
#[derive(Clone)]
pub(crate) struct Describer {
    conn: Arc<Mutex<Option<sqlx::PgConnection>>>,
    options: PgConnectOptions,
}

impl Describer {
    pub(crate) fn new(options: &PgConnectOptions, conn: Option<sqlx::PgConnection>) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
            options: options.clone().statement_cache_capacity(0),
        }
    }

    /// Result columns of `sql`, or `None` when it can't be described here (several statements,
    /// a temp table from another session, a dropped connection).
    pub(crate) async fn columns(&self, sql: String) -> Option<Vec<types::Column>> {
        let mut slot = self.conn.lock().await;
        if slot.is_none() {
            *slot = self.options.connect().await.ok();
        }
        let conn = slot.as_mut()?;
        match conn.prepare(AssertSqlSafe(sql).into_sql_str()).await {
            Ok(stmt) => Some(
                stmt.columns()
                    .iter()
                    .map(|c| types::Column::new(c.name(), c.type_info().name().to_lowercase()))
                    .collect(),
            ),
            Err(sqlx::Error::Database(_)) => None,
            Err(_) => {
                *slot = None;
                None
            }
        }
    }
}

impl Connection for PgConnection {
    fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        async move {
            let started = Instant::now();
            sqlx::query("SELECT 1")
                .execute(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(started.elapsed())
        }
        .boxed()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), Error>> {
        async move {
            self.pool.close().await;
            Ok(())
        }
        .boxed()
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

    fn backup(&self) -> Option<&dyn Backup> {
        Some(self)
    }

    fn aggregate(&self) -> Option<&dyn Aggregate> {
        Some(self)
    }

    fn column_stats(&self) -> Option<&dyn ColumnStats> {
        Some(self)
    }

    fn edit_rows(&self) -> Option<&dyn EditRows> {
        Some(self)
    }

    fn edit_schema(&self) -> Option<&dyn EditSchema> {
        Some(self)
    }

    fn transactions(&self) -> Option<&dyn Transactions> {
        Some(self)
    }
}
