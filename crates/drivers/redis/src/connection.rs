//! The Redis connection and its capability accessors.

use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::BoxFuture;
use redis::aio::ConnectionManager;
use traits::{Connection, KeyExpiry, KeyValue, MemoryStats, Transactions};
use types::{ConnectionInfo, Error};

/// A managed (auto-reconnecting) connection to one Redis database.
pub(crate) struct RedisConnection {
    pub(crate) conn: ConnectionManager,
    pub(crate) info: ConnectionInfo,
}

pub(crate) fn map(e: redis::RedisError) -> Error {
    if e.is_connection_dropped() || e.is_io_error() || e.is_timeout() {
        Error::connect(e.to_string())
    } else {
        Error::query(e.to_string())
    }
}

impl Connection for RedisConnection {
    fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        async move {
            let started = Instant::now();
            let mut conn = self.conn.clone();
            redis::cmd("PING")
                .query_async::<String>(&mut conn)
                .await
                .map_err(map)?;
            Ok(started.elapsed())
        }
        .boxed()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), Error>> {
        async { Ok(()) }.boxed()
    }

    fn key_value(&self) -> Option<&dyn KeyValue> {
        Some(self)
    }

    fn key_expiry(&self) -> Option<&dyn KeyExpiry> {
        Some(self)
    }

    fn memory_stats(&self) -> Option<&dyn MemoryStats> {
        Some(self)
    }

    fn transactions(&self) -> Option<&dyn Transactions> {
        Some(self)
    }
}
