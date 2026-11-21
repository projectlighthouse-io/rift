//! The Postgres connection and its capability accessors.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use traits::{
    Aggregate, Backup, Browse, ColumnStats, Connection, Describe, EditRows, EditSchema, Explain,
    Query, Spawner, Transactions,
};
use types::{ConnectionInfo, Error};

use crate::error;

/// A pooled Postgres connection.
pub(crate) struct PgConnection {
    pub(crate) pool: PgPool,
    /// Used to open a side connection for `pg_cancel_backend`.
    pub(crate) options: PgConnectOptions,
    pub(crate) spawner: Arc<dyn Spawner>,
    pub(crate) info: ConnectionInfo,
    /// For `pg_dump` and `psql`.
    pub(crate) password: Option<String>,
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
