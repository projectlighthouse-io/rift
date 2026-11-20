//! Browsing schemas and table rows.

use futures::future::BoxFuture;
use types::{DbObject, Error, Page, RowsRequest, SchemaName, TableRef};

/// Lists schemas and objects and reads table rows.
///
/// Implemented by: PostgreSQL, MySQL, SQLite, ClickHouse.
pub trait Browse: Send + Sync {
    /// Schemas (Postgres), databases (MySQL, ClickHouse) or `main` (SQLite).
    fn schemas(&self) -> BoxFuture<'_, Result<Vec<SchemaName>, Error>>;
    /// Schema shown first when the connection opens.
    fn default_schema(&self) -> BoxFuture<'_, Result<Option<SchemaName>, Error>>;
    /// Tables, views, functions and enums in a schema.
    fn objects<'a>(&'a self, schema: &'a SchemaName)
    -> BoxFuture<'a, Result<Vec<DbObject>, Error>>;
    /// A page of rows from a table, with columns (including key info).
    fn rows<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<Page, Error>>;
    /// Exact row count under the request's filters.
    fn count<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<u64, Error>>;
}
