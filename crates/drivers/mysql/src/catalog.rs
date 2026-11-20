//! Browse and Describe from `information_schema`. MySQL has no schema level: Rift's "schema" is
//! the database.

use dialect::{Dialect, rows};
use futures::future::BoxFuture;
use futures::{FutureExt, TryStreamExt};
use sqlx::{AssertSqlSafe, Column as _, Executor, SqlSafeStr, Statement, TypeInfo};
use traits::{Browse, Describe};
use types::{
    Column, ColumnDef, ColumnRef, DbObject, Error, ForeignKeyDef, IndexDef, ObjectKind, Page, Row,
    RowsRequest, SchemaName, TableDescription, TableRef,
};

use crate::connection::MySqlConnection;
use crate::error;
use crate::query::decode_row;

const SYSTEM: &str = "('mysql', 'information_schema', 'performance_schema', 'sys')";

/// name, column type, nullable, default, key, comment, fk schema, fk table, fk column.
type ColumnRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

const COLUMNS: &str = "\
    SELECT c.COLUMN_NAME, c.COLUMN_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, c.COLUMN_KEY, \
      c.COLUMN_COMMENT, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME \
    FROM information_schema.COLUMNS c \
    LEFT JOIN information_schema.KEY_COLUMN_USAGE k \
      ON k.TABLE_SCHEMA = c.TABLE_SCHEMA AND k.TABLE_NAME = c.TABLE_NAME \
      AND k.COLUMN_NAME = c.COLUMN_NAME AND k.REFERENCED_TABLE_NAME IS NOT NULL \
    WHERE c.TABLE_SCHEMA = ? AND c.TABLE_NAME = ? \
    ORDER BY c.ORDINAL_POSITION";

fn schema(table: &TableRef) -> String {
    table
        .schema
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default()
}

impl MySqlConnection {
    async fn column_defs(&self, table: &TableRef) -> Result<Vec<ColumnDef>, Error> {
        let rows: Vec<ColumnRow> = sqlx::query_as(COLUMNS)
            .bind(schema(table))
            .bind(table.name.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(error::map)?;
        if rows.is_empty() {
            return Err(Error::not_found(table.to_string()));
        }
        let mut out: Vec<ColumnDef> = Vec::new();
        for (name, ty, nullable, default, key, comment, fs, ft, fc) in rows {
            if out.iter().any(|c| c.name.as_str() == name) {
                continue;
            }
            out.push(ColumnDef {
                name: name.into(),
                type_name: ty,
                nullable: nullable == "YES",
                default,
                primary_key: key == "PRI",
                references: ft.zip(fc).map(|(t, c)| ColumnRef {
                    table: TableRef::new(fs.map(SchemaName::new), t),
                    column: c.into(),
                }),
                comment: (!comment.is_empty()).then_some(comment),
            });
        }
        Ok(out)
    }
}

impl Browse for MySqlConnection {
    fn schemas(&self) -> BoxFuture<'_, Result<Vec<SchemaName>, Error>> {
        async move {
            let sql = format!(
                "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME NOT IN {SYSTEM} ORDER BY 1"
            );
            let names: Vec<String> =
                sqlx::query_scalar(AssertSqlSafe(sql)).fetch_all(&self.pool).await.map_err(error::map)?;
            Ok(names.into_iter().map(SchemaName::new).collect())
        }
        .boxed()
    }

    fn default_schema(&self) -> BoxFuture<'_, Result<Option<SchemaName>, Error>> {
        async move {
            let current: Option<String> = sqlx::query_scalar("SELECT DATABASE()")
                .fetch_one(&self.pool)
                .await
                .map_err(error::map)?;
            match current {
                Some(db) => Ok(Some(SchemaName::new(db))),
                None => Ok(self.schemas().await?.into_iter().next()),
            }
        }
        .boxed()
    }

    fn objects<'a>(
        &'a self,
        schema: &'a SchemaName,
    ) -> BoxFuture<'a, Result<Vec<DbObject>, Error>> {
        async move {
            let rows: Vec<(String, String, Option<u64>)> = sqlx::query_as(
                "SELECT TABLE_NAME, TABLE_TYPE, TABLE_ROWS FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = ? \
                 UNION ALL \
                 SELECT ROUTINE_NAME, 'FUNCTION', NULL FROM information_schema.ROUTINES \
                 WHERE ROUTINE_SCHEMA = ? ORDER BY 1",
            )
            .bind(schema.as_str())
            .bind(schema.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(error::map)?;
            Ok(rows
                .into_iter()
                .map(|(name, kind, estimate)| DbObject {
                    table: TableRef::new(Some(schema.clone()), name),
                    kind: match kind.as_str() {
                        "VIEW" | "SYSTEM VIEW" => ObjectKind::View,
                        "FUNCTION" => ObjectKind::Function,
                        _ => ObjectKind::Table,
                    },
                    row_estimate: estimate,
                })
                .collect())
        }
        .boxed()
    }

    fn rows<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<Page, Error>> {
        async move {
            let defs = self.column_defs(table).await?;
            let sql = rows::select_rows(Dialect::MySql, table, request);
            let mut conn = self.pool.acquire().await.map_err(error::map)?;
            let stmt = (&mut *conn)
                .prepare(AssertSqlSafe(sql.clone()).into_sql_str())
                .await
                .map_err(error::map)?;
            let type_names: Vec<String> = stmt
                .columns()
                .iter()
                .map(|c| c.type_info().name().to_owned())
                .collect();
            let data: Vec<sqlx::mysql::MySqlRow> = sqlx::raw_sql(AssertSqlSafe(sql))
                .fetch(&mut *conn)
                .try_collect()
                .await
                .map_err(error::map)?;
            let columns = stmt
                .columns()
                .iter()
                .map(|c| {
                    let def = defs.iter().find(|d| d.name.as_str() == c.name());
                    Column {
                        name: c.name().into(),
                        type_name: def.map_or_else(
                            || c.type_info().name().to_lowercase(),
                            |d| d.type_name.clone(),
                        ),
                        nullable: def.map(|d| d.nullable),
                        primary_key: def.is_some_and(|d| d.primary_key),
                        references: def.and_then(|d| d.references.clone()),
                    }
                })
                .collect();
            Ok(Page {
                columns,
                rows: data
                    .iter()
                    .map(|r| Row {
                        values: decode_row(r, &type_names),
                    })
                    .collect(),
            })
        }
        .boxed()
    }

    fn count<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<u64, Error>> {
        async move {
            let sql = rows::count_rows(Dialect::MySql, table, request);
            let n: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
                .fetch_one(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(n as u64)
        }
        .boxed()
    }
}

impl Describe for MySqlConnection {
    fn describe<'a>(
        &'a self,
        table: &'a TableRef,
    ) -> BoxFuture<'a, Result<TableDescription, Error>> {
        async move {
            let columns = self.column_defs(table).await?;
            let stats: Vec<(String, i64, String, String, String)> = sqlx::query_as(
                "SELECT INDEX_NAME, NON_UNIQUE, INDEX_TYPE, COLUMN_NAME, COLLATION \
                 FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX",
            )
            .bind(schema(table))
            .bind(table.name.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(error::map)?;
            let mut indexes: Vec<IndexDef> = Vec::new();
            for (name, non_unique, method, column, collation) in stats {
                let column = if collation == "D" { format!("{column} DESC") } else { column };
                match indexes.iter_mut().find(|i| i.name.as_str() == name) {
                    Some(index) => index.columns.push(column),
                    None => indexes.push(IndexDef {
                        primary: name == "PRIMARY",
                        name: name.into(),
                        columns: vec![column],
                        method: Some(method.to_lowercase()),
                        unique: non_unique == 0,
                        size_bytes: None,
                    }),
                }
            }
            let fk_rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
                "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_NAME, \
                   k.REFERENCED_COLUMN_NAME, r.DELETE_RULE, r.UPDATE_RULE \
                 FROM information_schema.KEY_COLUMN_USAGE k \
                 JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
                   ON r.CONSTRAINT_SCHEMA = k.TABLE_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME \
                 WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? ORDER BY 1, k.ORDINAL_POSITION",
            )
            .bind(schema(table))
            .bind(table.name.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(error::map)?;
            let mut foreign_keys: Vec<ForeignKeyDef> = Vec::new();
            for (name, column, ref_table, ref_column, on_delete, on_update) in fk_rows {
                match foreign_keys.iter_mut().find(|f| f.name == name) {
                    Some(fk) => {
                        fk.columns.push(column.into());
                        fk.ref_columns.push(ref_column.into());
                    }
                    None => foreign_keys.push(ForeignKeyDef {
                        name,
                        columns: vec![column.into()],
                        ref_table: TableRef::new(table.schema.clone(), ref_table),
                        ref_columns: vec![ref_column.into()],
                        on_delete,
                        on_update,
                    }),
                }
            }
            let (rows, size): (Option<u64>, Option<u64>) = sqlx::query_as(
                "SELECT TABLE_ROWS, DATA_LENGTH + INDEX_LENGTH FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
            )
            .bind(schema(table))
            .bind(table.name.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(error::map)?;
            Ok(TableDescription { table: table.clone(), columns, indexes, foreign_keys, row_estimate: rows, size_bytes: size })
        }
        .boxed()
    }

    fn ddl<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<String, Error>> {
        async move {
            let sql = format!("SHOW CREATE TABLE {}", Dialect::MySql.table(table));
            let mut conn = self.pool.acquire().await.map_err(error::map)?;
            let rows: Vec<sqlx::mysql::MySqlRow> = sqlx::raw_sql(AssertSqlSafe(sql))
                .fetch(&mut *conn)
                .try_collect()
                .await
                .map_err(error::map)?;
            let row = rows
                .first()
                .ok_or_else(|| Error::not_found(table.to_string()))?;
            match decode_row(row, &[]).get(1) {
                Some(v) => Ok(format!("{v};")),
                None => Err(Error::not_found(table.to_string())),
            }
        }
        .boxed()
    }
}
