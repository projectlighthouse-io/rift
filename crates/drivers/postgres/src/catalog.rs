//! Browse and Describe from the Postgres catalogs.

use dialect::{Dialect, rows};
use futures::future::BoxFuture;
use futures::{FutureExt, TryStreamExt};
use sqlx::{AssertSqlSafe, Column as _, Executor, SqlSafeStr, Statement, TypeInfo};
use traits::{Browse, Describe};
use types::{
    Column, ColumnDef, ColumnRef, DbObject, Error, ForeignKeyDef, IndexDef, ObjectKind, Page, Row,
    RowsRequest, SchemaName, TableDescription, TableRef,
};

use crate::connection::PgConnection;
use crate::error;
use crate::query::decode_row;

const SCHEMAS: &str = "SELECT nspname::text FROM pg_namespace \
    WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' ORDER BY 1";

const OBJECTS: &str = "\
    SELECT c.relname::text, c.relkind::text, GREATEST(c.reltuples, 0)::int8 \
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
    WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm') \
    UNION ALL \
    SELECT DISTINCT p.proname::text, 'f', NULL::int8 \
    FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
    WHERE n.nspname = $1 AND p.prokind IN ('f', 'p') \
    UNION ALL \
    SELECT t.typname::text, 'e', NULL::int8 \
    FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
    WHERE n.nspname = $1 AND t.typtype = 'e' \
    ORDER BY 1";

/// name, type, nullable, default, comment, is_pk, fk schema, fk table, fk column.
type ColumnRow = (
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// name, columns, ref schema, ref table, ref columns, on delete, on update.
type FkRow = (
    String,
    Vec<String>,
    String,
    String,
    Vec<String>,
    String,
    String,
);

const COLUMNS: &str = "\
    SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
      pg_get_expr(d.adbin, d.adrelid), col_description(a.attrelid, a.attnum), \
      EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary \
              AND a.attnum = ANY(i.indkey)), \
      fk.nspname::text, fk.relname::text, fk.attname::text \
    FROM pg_attribute a \
    LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
    LEFT JOIN LATERAL ( \
      SELECT fn.nspname, fc.relname, fa.attname \
      FROM pg_constraint c \
      JOIN pg_class fc ON fc.oid = c.confrelid \
      JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
      JOIN pg_attribute fa ON fa.attrelid = c.confrelid \
        AND fa.attnum = c.confkey[array_position(c.conkey, a.attnum)] \
      WHERE c.conrelid = a.attrelid AND c.contype = 'f' AND a.attnum = ANY(c.conkey) \
      LIMIT 1) fk ON TRUE \
    WHERE a.attrelid = to_regclass($1) AND a.attnum > 0 AND NOT a.attisdropped \
    ORDER BY a.attnum";

const INDEXES: &str = "\
    SELECT ic.relname::text, i.indisunique, i.indisprimary, am.amname::text, \
      pg_relation_size(i.indexrelid)::int8, \
      ARRAY(SELECT pg_get_indexdef(i.indexrelid, k + 1, true) \
              || CASE WHEN i.indoption[k] & 1 = 1 THEN ' DESC' ELSE '' END \
            FROM generate_subscripts(i.indkey, 1) k ORDER BY k) \
    FROM pg_index i \
    JOIN pg_class ic ON ic.oid = i.indexrelid \
    JOIN pg_am am ON am.oid = ic.relam \
    WHERE i.indrelid = to_regclass($1) \
    ORDER BY i.indisprimary DESC, ic.relname";

const FOREIGN_KEYS: &str = "\
    SELECT c.conname::text, \
      ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(n, o) \
            JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.n ORDER BY k.o), \
      fn.nspname::text, fc.relname::text, \
      ARRAY(SELECT a.attname::text FROM unnest(c.confkey) WITH ORDINALITY k(n, o) \
            JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.n ORDER BY k.o), \
      c.confdeltype::text, c.confupdtype::text \
    FROM pg_constraint c \
    JOIN pg_class fc ON fc.oid = c.confrelid \
    JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
    WHERE c.conrelid = to_regclass($1) AND c.contype = 'f' ORDER BY 1";

const TABLE_STATS: &str = "SELECT GREATEST(c.reltuples, 0)::int8, pg_total_relation_size(c.oid)::int8 \
    FROM pg_class c WHERE c.oid = to_regclass($1)";

fn fk_action(code: &str) -> String {
    match code {
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "NO ACTION",
    }
    .into()
}

/// Shortens `format_type` output to the names the UI shows (`char(3)`, `timestamptz`).
fn short_type(name: &str) -> String {
    let replacements = [
        ("character varying", "varchar"),
        ("character", "char"),
        ("timestamp with time zone", "timestamptz"),
        ("timestamp without time zone", "timestamp"),
        ("time without time zone", "time"),
        ("double precision", "float8"),
    ];
    replacements
        .iter()
        .find_map(|(long, short)| name.strip_prefix(long).map(|rest| format!("{short}{rest}")))
        .unwrap_or_else(|| name.to_owned())
}

impl PgConnection {
    async fn column_defs(&self, table: &TableRef) -> Result<Vec<ColumnDef>, Error> {
        let quoted = Dialect::Postgres.table(table);
        let rows: Vec<ColumnRow> = sqlx::query_as(COLUMNS)
            .bind(&quoted)
            .fetch_all(&self.pool)
            .await
            .map_err(error::map)?;
        if rows.is_empty() {
            return Err(Error::not_found(table.to_string()));
        }
        Ok(rows
            .into_iter()
            .map(
                |(name, ty, nullable, default, comment, pk, fk_schema, fk_table, fk_column)| {
                    ColumnDef {
                        name: name.into(),
                        type_name: short_type(&ty),
                        nullable,
                        default,
                        primary_key: pk,
                        references: fk_table.zip(fk_column).map(|(t, c)| ColumnRef {
                            table: TableRef::new(fk_schema.map(SchemaName::new), t),
                            column: c.into(),
                        }),
                        comment,
                    }
                },
            )
            .collect())
    }
}

impl Browse for PgConnection {
    fn schemas(&self) -> BoxFuture<'_, Result<Vec<SchemaName>, Error>> {
        async move {
            let names: Vec<String> = sqlx::query_scalar(SCHEMAS)
                .fetch_all(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(names.into_iter().map(SchemaName::new).collect())
        }
        .boxed()
    }

    fn default_schema(&self) -> BoxFuture<'_, Result<Option<SchemaName>, Error>> {
        async move {
            let name: Option<String> = sqlx::query_scalar("SELECT current_schema()::text")
                .fetch_one(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(name.map(SchemaName::new))
        }
        .boxed()
    }

    fn objects<'a>(
        &'a self,
        schema: &'a SchemaName,
    ) -> BoxFuture<'a, Result<Vec<DbObject>, Error>> {
        async move {
            let rows: Vec<(String, String, Option<i64>)> = sqlx::query_as(OBJECTS)
                .bind(schema.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(rows
                .into_iter()
                .map(|(name, kind, estimate)| DbObject {
                    table: TableRef::new(Some(schema.clone()), name),
                    kind: match kind.as_str() {
                        "v" => ObjectKind::View,
                        "m" => ObjectKind::MaterializedView,
                        "f" => ObjectKind::Function,
                        "e" => ObjectKind::Enum,
                        _ => ObjectKind::Table,
                    },
                    row_estimate: estimate.map(|n| n as u64),
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
            let sql = rows::select_rows(Dialect::Postgres, table, request);
            let mut conn = self.pool.acquire().await.map_err(error::map)?;
            let described = (&mut *conn)
                .prepare(AssertSqlSafe(sql.clone()).into_sql_str())
                .await
                .map_err(error::map)?;
            let type_names: Vec<String> = described
                .columns()
                .iter()
                .map(|c| c.type_info().name().to_lowercase())
                .collect();
            let data: Vec<sqlx::postgres::PgRow> = sqlx::raw_sql(AssertSqlSafe(sql))
                .fetch(&mut *conn)
                .try_collect()
                .await
                .map_err(error::map)?;
            let columns = described
                .columns()
                .iter()
                .zip(&type_names)
                .map(|(c, ty)| {
                    let def = defs.iter().find(|d| d.name.as_str() == c.name());
                    let shown = match def {
                        Some(d) if d.type_name.starts_with("char(") => d.type_name.clone(),
                        _ => ty.clone(),
                    };
                    Column {
                        name: c.name().into(),
                        type_name: shown,
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
            let sql = rows::count_rows(Dialect::Postgres, table, request);
            let n: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
                .fetch_one(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(n as u64)
        }
        .boxed()
    }
}

impl Describe for PgConnection {
    fn describe<'a>(
        &'a self,
        table: &'a TableRef,
    ) -> BoxFuture<'a, Result<TableDescription, Error>> {
        async move {
            let quoted = Dialect::Postgres.table(table);
            let columns = self.column_defs(table).await?;
            let index_rows: Vec<(String, bool, bool, String, i64, Vec<String>)> =
                sqlx::query_as(INDEXES)
                    .bind(&quoted)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(error::map)?;
            let fk_rows: Vec<FkRow> = sqlx::query_as(FOREIGN_KEYS)
                .bind(&quoted)
                .fetch_all(&self.pool)
                .await
                .map_err(error::map)?;
            let (row_estimate, size): (i64, i64) = sqlx::query_as(TABLE_STATS)
                .bind(&quoted)
                .fetch_one(&self.pool)
                .await
                .map_err(error::map)?;
            Ok(TableDescription {
                table: table.clone(),
                columns,
                indexes: index_rows
                    .into_iter()
                    .map(|(name, unique, primary, method, size, columns)| IndexDef {
                        name: name.into(),
                        columns,
                        method: Some(method),
                        unique,
                        primary,
                        size_bytes: Some(size as u64),
                    })
                    .collect(),
                foreign_keys: fk_rows
                    .into_iter()
                    .map(
                        |(name, cols, schema, ref_table, ref_cols, del, upd)| ForeignKeyDef {
                            name,
                            columns: cols.into_iter().map(Into::into).collect(),
                            ref_table: TableRef::new(Some(SchemaName::new(schema)), ref_table),
                            ref_columns: ref_cols.into_iter().map(Into::into).collect(),
                            on_delete: fk_action(&del),
                            on_update: fk_action(&upd),
                        },
                    )
                    .collect(),
                row_estimate: Some(row_estimate as u64),
                size_bytes: Some(size as u64),
            })
        }
        .boxed()
    }

    fn ddl<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<String, Error>> {
        async move {
            let desc = self.describe(table).await?;
            let quoted = Dialect::Postgres.table(table);
            let ident = |s: &str| Dialect::Postgres.ident(s);
            let mut lines: Vec<String> = desc
                .columns
                .iter()
                .map(|c| {
                    let mut line = format!("  {} {}", ident(c.name.as_str()), c.type_name);
                    if let Some(default) = &c.default {
                        line.push_str(&format!(" DEFAULT {default}"));
                    }
                    if !c.nullable {
                        line.push_str(" NOT NULL");
                    }
                    line
                })
                .collect();
            let pk: Vec<String> = desc
                .primary_key()
                .iter()
                .map(|c| ident(c.as_str()))
                .collect();
            if !pk.is_empty() {
                lines.push(format!("  PRIMARY KEY ({})", pk.join(", ")));
            }
            for fk in &desc.foreign_keys {
                let cols: Vec<String> = fk.columns.iter().map(|c| ident(c.as_str())).collect();
                let refs: Vec<String> = fk.ref_columns.iter().map(|c| ident(c.as_str())).collect();
                lines.push(format!(
                    "  CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {}",
                    ident(&fk.name),
                    cols.join(", "),
                    Dialect::Postgres.table(&fk.ref_table),
                    refs.join(", "),
                    fk.on_delete,
                    fk.on_update
                ));
            }
            let mut ddl = format!("CREATE TABLE {quoted} (\n{}\n);", lines.join(",\n"));
            let defs: Vec<String> = sqlx::query_scalar(
                "SELECT pg_get_indexdef(indexrelid) FROM pg_index \
                 WHERE indrelid = to_regclass($1) AND NOT indisprimary ORDER BY 1",
            )
            .bind(&quoted)
            .fetch_all(&self.pool)
            .await
            .map_err(error::map)?;
            for def in defs {
                ddl.push_str(&format!("\n{def};"));
            }
            Ok(ddl)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortens_type_names() {
        assert_eq!(super::short_type("character(3)"), "char(3)");
        assert_eq!(super::short_type("timestamp with time zone"), "timestamptz");
        assert_eq!(super::short_type("numeric(10,2)"), "numeric(10,2)");
    }
}
