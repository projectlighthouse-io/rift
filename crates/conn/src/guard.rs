//! Read-only safe mode as a wrapper around any connection. Capabilities that only write
//! disappear; capabilities that mix reads and writes keep their reads and refuse their writes;
//! SQL is checked by `dialect::safety` before it reaches the server. Drivers that can also
//! enforce read-only in the database do so too (defence in depth).

use std::sync::Arc;
use std::time::Duration;

use dialect::{Dialect, safety};
use futures::FutureExt;
use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;
use traits::{
    Aggregate, Browse, ColumnStats, Connection, Describe, Explain, KeyDirectories, KeyEncoding,
    KeyExpiry, KeyValue, MemoryStats, Query, StatementOutput, TableTtl,
};
use types::{
    AggregateRequest, ChartData, ChartSource, ConnectionInfo, Error, ExplainPlan, KeyBytes,
    KeyPage, KeyScan, KvValue, TableRef, TtlRule,
};

fn refused(what: &str) -> Error {
    Error::ReadOnly {
        reason: format!("{what} is blocked on this connection"),
    }
}

/// A connection in read-only safe mode.
pub struct ReadOnlyConnection {
    inner: Arc<dyn Connection>,
    dialect: Option<Dialect>,
}

impl ReadOnlyConnection {
    /// Wraps `inner`; `driver` picks the SQL dialect used to check statements.
    #[must_use]
    pub fn new(inner: Arc<dyn Connection>, driver: &str) -> Self {
        Self {
            inner,
            dialect: Dialect::for_driver(driver),
        }
    }

    fn check_sql(&self, sql: &str) -> Result<(), Error> {
        if safety::writes(self.dialect, sql) {
            Err(refused("a statement that may write"))
        } else {
            Ok(())
        }
    }
}

impl Connection for ReadOnlyConnection {
    fn info(&self) -> &ConnectionInfo {
        self.inner.info()
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        self.inner.ping()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), Error>> {
        self.inner.close()
    }

    fn browse(&self) -> Option<&dyn Browse> {
        self.inner.browse()
    }

    fn describe(&self) -> Option<&dyn Describe> {
        self.inner.describe()
    }

    fn column_stats(&self) -> Option<&dyn ColumnStats> {
        self.inner.column_stats()
    }

    fn memory_stats(&self) -> Option<&dyn MemoryStats> {
        self.inner.memory_stats()
    }

    fn key_directories(&self) -> Option<&dyn KeyDirectories> {
        self.inner.key_directories()
    }

    fn key_encoding(&self) -> Option<&dyn KeyEncoding> {
        self.inner.key_encoding()
    }

    fn query(&self) -> Option<&dyn Query> {
        self.inner.query().map(|_| self as &dyn Query)
    }

    fn explain(&self) -> Option<&dyn Explain> {
        self.inner.explain().map(|_| self as &dyn Explain)
    }

    fn aggregate(&self) -> Option<&dyn Aggregate> {
        self.inner.aggregate().map(|_| self as &dyn Aggregate)
    }

    fn key_value(&self) -> Option<&dyn KeyValue> {
        self.inner.key_value().map(|_| self as &dyn KeyValue)
    }

    fn key_expiry(&self) -> Option<&dyn KeyExpiry> {
        self.inner.key_expiry().map(|_| self as &dyn KeyExpiry)
    }

    fn table_ttl(&self) -> Option<&dyn TableTtl> {
        self.inner.table_ttl().map(|_| self as &dyn TableTtl)
    }
    // edit_rows, edit_schema, transactions and backup stay `None`: they only write (backup
    // shares its trait with restore).
}

/// The wrapped capability; present because the wrapper only exposes what `inner` has.
macro_rules! inner {
    ($self:ident. $cap:ident) => {
        $self
            .inner
            .$cap()
            .ok_or_else(|| Error::unsupported(stringify!($cap)))?
    };
}

impl Query for ReadOnlyConnection {
    fn execute<'a>(
        &'a self,
        sql: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<StatementOutput, Error>> {
        async move {
            self.check_sql(sql)?;
            inner!(self.query).execute(sql, cancel).await
        }
        .boxed()
    }
}

impl Explain for ReadOnlyConnection {
    fn explain<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<ExplainPlan, Error>> {
        async move {
            self.check_sql(sql)?;
            inner!(self.explain).explain(sql).await
        }
        .boxed()
    }
}

impl Aggregate for ReadOnlyConnection {
    fn aggregate<'a>(
        &'a self,
        request: &'a AggregateRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChartData, Error>> {
        async move {
            if let ChartSource::Query(sql) = &request.source {
                self.check_sql(sql)?;
            }
            inner!(self.aggregate).aggregate(request, cancel).await
        }
        .boxed()
    }
}

impl KeyValue for ReadOnlyConnection {
    fn scan<'a>(&'a self, scan: &'a KeyScan) -> BoxFuture<'a, Result<KeyPage, Error>> {
        async move { inner!(self.key_value).scan(scan).await }.boxed()
    }

    fn get<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<KvValue>, Error>> {
        async move { inner!(self.key_value).get(key).await }.boxed()
    }

    fn set<'a>(&'a self, _: &'a KeyBytes, _: &'a KvValue) -> BoxFuture<'a, Result<(), Error>> {
        async { Err(refused("writing a key")) }.boxed()
    }

    fn delete<'a>(&'a self, _: &'a KeyBytes) -> BoxFuture<'a, Result<bool, Error>> {
        async { Err(refused("deleting a key")) }.boxed()
    }

    fn rename<'a>(&'a self, _: &'a KeyBytes, _: &'a KeyBytes) -> BoxFuture<'a, Result<(), Error>> {
        async { Err(refused("renaming a key")) }.boxed()
    }

    fn key_count(&self) -> BoxFuture<'_, Result<u64, Error>> {
        async move { inner!(self.key_value).key_count().await }.boxed()
    }
}

impl KeyExpiry for ReadOnlyConnection {
    fn ttl<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<Duration>, Error>> {
        async move { inner!(self.key_expiry).ttl(key).await }.boxed()
    }

    fn set_ttl<'a>(
        &'a self,
        _: &'a KeyBytes,
        _: Option<Duration>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async { Err(refused("changing a TTL")) }.boxed()
    }
}

impl TableTtl for ReadOnlyConnection {
    fn ttl_rules<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<Vec<TtlRule>, Error>> {
        async move { inner!(self.table_ttl).ttl_rules(table).await }.boxed()
    }

    fn set_table_ttl<'a>(
        &'a self,
        _: &'a TableRef,
        _: Option<&'a TtlRule>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async { Err(refused("changing a table TTL")) }.boxed()
    }
}
