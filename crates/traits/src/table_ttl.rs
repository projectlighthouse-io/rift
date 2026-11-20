//! Table TTL rules.

use futures::future::BoxFuture;
use types::{Error, TableRef, TtlRule};

/// Reads and changes table/column TTL rules.
///
/// Implemented by: ClickHouse.
pub trait TableTtl: Send + Sync {
    /// Current rules.
    fn ttl_rules<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<Vec<TtlRule>, Error>>;
    /// Sets the table-level rule (`column == None`), or removes it with `None`.
    fn set_table_ttl<'a>(
        &'a self,
        table: &'a TableRef,
        rule: Option<&'a TtlRule>,
    ) -> BoxFuture<'a, Result<(), Error>>;
}
