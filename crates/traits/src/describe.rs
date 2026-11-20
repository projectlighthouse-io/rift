//! Table structure.

use futures::future::BoxFuture;
use types::{Error, TableDescription, TableRef};

/// Describes columns, indexes, foreign keys and DDL.
///
/// Implemented by: PostgreSQL, MySQL, SQLite, ClickHouse.
pub trait Describe: Send + Sync {
    /// Structure of a table or view.
    fn describe<'a>(
        &'a self,
        table: &'a TableRef,
    ) -> BoxFuture<'a, Result<TableDescription, Error>>;
    /// `CREATE` statement for the object.
    fn ddl<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<String, Error>>;
}
