//! Row edits, schema changes and atomic batches, each in one transaction.

use dialect::{Dialect, ddl, edits};
use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{EditRows, EditSchema, Transactions};
use types::{Error, RowEdit, SchemaChange, TableRef, TxOp};

use crate::connection::SqliteConnection;
use crate::error;

/// Runs statements in one transaction; `expect_one[i]` requires statement `i` to change one row.
fn run_all(conn: &mut rusqlite::Connection, statements: &[(String, bool)]) -> Result<u64, Error> {
    let tx = conn.transaction().map_err(error::map)?;
    let mut changed = 0;
    for (sql, one_row) in statements {
        let n = tx.execute(sql, []).map_err(error::map)? as u64;
        if *one_row && n != 1 {
            return Err(Error::query(
                "a row was changed or deleted by someone else; reload and try again",
            ));
        }
        changed += n;
    }
    tx.commit().map_err(error::map)?;
    Ok(changed)
}

impl EditRows for SqliteConnection {
    fn apply<'a>(&'a self, edits: &'a [RowEdit]) -> BoxFuture<'a, Result<u64, Error>> {
        async move {
            let statements = edits
                .iter()
                .map(|e| {
                    Ok((
                        edits::edit_sql(Dialect::Sqlite, e)?,
                        edits::expects_one_row(e),
                    ))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            self.with(move |c| run_all(c, &statements)).await
        }
        .boxed()
    }
}

impl EditSchema for SqliteConnection {
    fn preview(&self, table: &TableRef, changes: &[SchemaChange]) -> Result<Vec<String>, Error> {
        ddl::schema_changes_sql(Dialect::Sqlite, table, changes)
    }

    fn apply_schema<'a>(
        &'a self,
        table: &'a TableRef,
        changes: &'a [SchemaChange],
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let statements: Vec<(String, bool)> = self
                .preview(table, changes)?
                .into_iter()
                .map(|s| (s, false))
                .collect();
            self.with(move |c| run_all(c, &statements).map(|_| ()))
                .await
        }
        .boxed()
    }
}

impl Transactions for SqliteConnection {
    fn atomic<'a>(&'a self, ops: &'a [TxOp]) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let statements = ops
                .iter()
                .map(|op| match op {
                    TxOp::Sql { sql } => Ok((sql.clone(), false)),
                    _ => Err(Error::unsupported("key-value op on SQLite")),
                })
                .collect::<Result<Vec<_>, Error>>()?;
            self.with(move |c| run_all(c, &statements).map(|_| ()))
                .await
        }
        .boxed()
    }
}
