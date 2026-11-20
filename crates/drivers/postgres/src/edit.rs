//! Row edits and atomic batches, each in one transaction.

use dialect::{Dialect, ddl, edits};
use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::AssertSqlSafe;
use traits::{EditRows, EditSchema, Transactions};
use types::{Error, RowEdit, SchemaChange, TableRef, TxOp};

use crate::connection::PgConnection;
use crate::error;

impl EditRows for PgConnection {
    fn apply<'a>(&'a self, edits: &'a [RowEdit]) -> BoxFuture<'a, Result<u64, Error>> {
        async move {
            let statements = edits
                .iter()
                .map(|e| {
                    Ok((
                        edits::edit_sql(Dialect::Postgres, e)?,
                        edits::expects_one_row(e),
                    ))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let mut tx = self.pool.begin().await.map_err(error::map)?;
            let mut changed = 0;
            for (sql, one_row) in statements {
                let result = sqlx::raw_sql(AssertSqlSafe(sql))
                    .execute(&mut *tx)
                    .await
                    .map_err(error::map)?;
                if one_row && result.rows_affected() != 1 {
                    return Err(Error::query(
                        "a row was changed or deleted by someone else; reload and try again",
                    ));
                }
                changed += result.rows_affected();
            }
            tx.commit().await.map_err(error::map)?;
            Ok(changed)
        }
        .boxed()
    }
}

impl Transactions for PgConnection {
    fn atomic<'a>(&'a self, ops: &'a [TxOp]) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let statements = ops
                .iter()
                .map(|op| match op {
                    TxOp::Sql { sql } => Ok(sql.clone()),
                    _ => Err(Error::unsupported("key-value op on PostgreSQL")),
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let mut tx = self.pool.begin().await.map_err(error::map)?;
            for sql in statements {
                sqlx::raw_sql(AssertSqlSafe(sql))
                    .execute(&mut *tx)
                    .await
                    .map_err(error::map)?;
            }
            tx.commit().await.map_err(error::map)
        }
        .boxed()
    }
}

impl EditSchema for PgConnection {
    fn preview(&self, table: &TableRef, changes: &[SchemaChange]) -> Result<Vec<String>, Error> {
        ddl::schema_changes_sql(Dialect::Postgres, table, changes)
    }

    fn apply_schema<'a>(
        &'a self,
        table: &'a TableRef,
        changes: &'a [SchemaChange],
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let statements = self.preview(table, changes)?;
            let mut tx = self.pool.begin().await.map_err(error::map)?;
            for sql in statements {
                sqlx::raw_sql(AssertSqlSafe(sql))
                    .execute(&mut *tx)
                    .await
                    .map_err(error::map)?;
            }
            tx.commit().await.map_err(error::map)
        }
        .boxed()
    }
}
