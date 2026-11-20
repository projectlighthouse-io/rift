//! Browse and Describe from `sqlite_master` and PRAGMAs.

use dialect::{Dialect, rows};
use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{Browse, Describe};
use types::{
    Column, ColumnDef, ColumnRef, DbObject, Error, ForeignKeyDef, IndexDef, ObjectKind, Page, Row,
    RowsRequest, SchemaName, TableDescription, TableRef,
};

use crate::connection::SqliteConnection;
use crate::{error, values};

fn schema_of(table: &TableRef) -> String {
    table
        .schema
        .as_ref()
        .map_or_else(|| "main".to_owned(), ToString::to_string)
}

fn pragma(table: &TableRef, name: &str) -> String {
    let d = Dialect::Sqlite;
    format!(
        "PRAGMA {}.{name}({})",
        d.ident(&schema_of(table)),
        d.string(table.name.as_str())
    )
}

fn action(text: &str) -> String {
    if text.is_empty() {
        "NO ACTION".into()
    } else {
        text.to_owned()
    }
}

/// Column definitions, keys and foreign keys of a table.
pub(crate) fn column_defs(
    conn: &rusqlite::Connection,
    table: &TableRef,
) -> Result<(Vec<ColumnDef>, Vec<ForeignKeyDef>), Error> {
    let mut stmt = conn
        .prepare(&pragma(table, "foreign_key_list"))
        .map_err(error::map)?;
    let fk_rows: Vec<(i64, String, String, Option<String>, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .map_err(error::map)?
        .collect::<Result<_, _>>()
        .map_err(error::map)?;
    let mut foreign_keys: Vec<ForeignKeyDef> = Vec::new();
    for (id, target, from, to, on_update, on_delete) in &fk_rows {
        let to = to.clone().unwrap_or_else(|| "rowid".into());
        match foreign_keys
            .iter_mut()
            .find(|f| f.name == format!("fk_{id}"))
        {
            Some(fk) => {
                fk.columns.push(from.as_str().into());
                fk.ref_columns.push(to.into());
            }
            None => foreign_keys.push(ForeignKeyDef {
                name: format!("fk_{id}"),
                columns: vec![from.as_str().into()],
                ref_table: TableRef::new(table.schema.clone(), target.as_str()),
                ref_columns: vec![to.into()],
                on_delete: action(on_delete),
                on_update: action(on_update),
            }),
        }
    }
    let mut stmt = conn
        .prepare(&pragma(table, "table_info"))
        .map_err(error::map)?;
    let columns: Vec<ColumnDef> = stmt
        .query_map([], |r| {
            let name: String = r.get(1)?;
            let type_name: String = r.get(2)?;
            let notnull: bool = r.get(3)?;
            let default: Option<String> = r.get(4)?;
            let pk: i64 = r.get(5)?;
            Ok((name, type_name, notnull, default, pk))
        })
        .map_err(error::map)?
        .map(|row| {
            let (name, type_name, notnull, default, pk) = row.map_err(error::map)?;
            let references = foreign_keys.iter().find_map(|fk| {
                fk.columns
                    .iter()
                    .position(|c| c.as_str() == name)
                    .map(|i| ColumnRef {
                        table: fk.ref_table.clone(),
                        column: fk.ref_columns[i].clone(),
                    })
            });
            Ok(ColumnDef {
                name: name.into(),
                type_name: type_name.to_lowercase(),
                nullable: !notnull && pk == 0,
                default,
                primary_key: pk > 0,
                references,
                comment: None,
            })
        })
        .collect::<Result<_, Error>>()?;
    if columns.is_empty() {
        return Err(Error::not_found(table.to_string()));
    }
    Ok((columns, foreign_keys))
}

impl Browse for SqliteConnection {
    fn schemas(&self) -> BoxFuture<'_, Result<Vec<SchemaName>, Error>> {
        async move {
            self.with(|c| {
                let mut stmt = c.prepare("PRAGMA database_list").map_err(error::map)?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(1))
                    .map_err(error::map)?
                    .filter_map(Result::ok)
                    .filter(|n| n != "temp")
                    .map(SchemaName::new)
                    .collect();
                Ok(names)
            })
            .await
        }
        .boxed()
    }

    fn default_schema(&self) -> BoxFuture<'_, Result<Option<SchemaName>, Error>> {
        async { Ok(Some(SchemaName::new("main"))) }.boxed()
    }

    fn objects<'a>(
        &'a self,
        schema: &'a SchemaName,
    ) -> BoxFuture<'a, Result<Vec<DbObject>, Error>> {
        let schema = schema.clone();
        async move {
            self.with(move |c| {
                let sql = format!(
                    "SELECT name, type FROM {}.sqlite_master \
                     WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                     ORDER BY name",
                    Dialect::Sqlite.ident(schema.as_str())
                );
                let mut stmt = c.prepare(&sql).map_err(error::map)?;
                let objects = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(error::map)?
                    .filter_map(Result::ok)
                    .map(|(name, kind)| DbObject {
                        table: TableRef::new(Some(schema.clone()), name),
                        kind: if kind == "view" {
                            ObjectKind::View
                        } else {
                            ObjectKind::Table
                        },
                        row_estimate: None,
                    })
                    .collect();
                Ok(objects)
            })
            .await
        }
        .boxed()
    }

    fn rows<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<Page, Error>> {
        let (table, request) = (table.clone(), request.clone());
        async move {
            self.with(move |c| {
                let defs = column_defs(c, &table)
                    .map(|(cols, _)| cols)
                    .unwrap_or_default();
                let sql = rows::select_rows(Dialect::Sqlite, &table, &request);
                let mut stmt = c.prepare(&sql).map_err(error::map)?;
                let decls: Vec<Option<String>> = stmt
                    .columns()
                    .iter()
                    .map(|c| c.decl_type().map(str::to_owned))
                    .collect();
                let columns: Vec<Column> = stmt
                    .column_names()
                    .iter()
                    .zip(&decls)
                    .map(|(name, decl)| {
                        let def = defs.iter().find(|d| d.name.as_str() == *name);
                        Column {
                            name: (*name).into(),
                            type_name: decl.clone().unwrap_or_default().to_lowercase(),
                            nullable: def.map(|d| d.nullable),
                            primary_key: def.is_some_and(|d| d.primary_key),
                            references: def.and_then(|d| d.references.clone()),
                        }
                    })
                    .collect();
                let mut rows = stmt.query([]).map_err(error::map)?;
                let mut out = Vec::new();
                while let Some(row) = rows.next().map_err(error::map)? {
                    out.push(Row {
                        values: (0..columns.len())
                            .map(|i| values::value(row.get_ref_unwrap(i), decls[i].as_deref()))
                            .collect(),
                    });
                }
                Ok(Page { columns, rows: out })
            })
            .await
        }
        .boxed()
    }

    fn count<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<u64, Error>> {
        let sql = rows::count_rows(Dialect::Sqlite, table, request);
        async move {
            self.with(move |c| {
                c.query_row(&sql, [], |r| r.get::<_, i64>(0))
                    .map(|n| n as u64)
                    .map_err(error::map)
            })
            .await
        }
        .boxed()
    }
}

impl Describe for SqliteConnection {
    fn describe<'a>(
        &'a self,
        table: &'a TableRef,
    ) -> BoxFuture<'a, Result<TableDescription, Error>> {
        let table = table.clone();
        async move {
            self.with(move |c| {
                let (columns, foreign_keys) = column_defs(c, &table)?;
                let mut stmt = c
                    .prepare(&pragma(&table, "index_list"))
                    .map_err(error::map)?;
                let list: Vec<(String, bool, String)> = stmt
                    .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?)))
                    .map_err(error::map)?
                    .filter_map(Result::ok)
                    .collect();
                let mut indexes = Vec::new();
                for (name, unique, origin) in list {
                    let info = format!(
                        "PRAGMA {}.index_info({})",
                        Dialect::Sqlite.ident(&schema_of(&table)),
                        Dialect::Sqlite.string(&name)
                    );
                    let mut stmt = c.prepare(&info).map_err(error::map)?;
                    let cols: Vec<String> = stmt
                        .query_map([], |r| r.get::<_, Option<String>>(2))
                        .map_err(error::map)?
                        .filter_map(Result::ok)
                        .map(Option::unwrap_or_default)
                        .collect();
                    indexes.push(IndexDef {
                        name: name.into(),
                        columns: cols,
                        method: Some("btree".into()),
                        unique,
                        primary: origin == "pk",
                        size_bytes: None,
                    });
                }
                let rows = c
                    .query_row(
                        &rows::count_rows(Dialect::Sqlite, &table, &RowsRequest::default()),
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .ok()
                    .map(|n| n as u64);
                Ok(TableDescription {
                    table: table.clone(),
                    columns,
                    indexes,
                    foreign_keys,
                    row_estimate: rows,
                    size_bytes: None,
                })
            })
            .await
        }
        .boxed()
    }

    fn ddl<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<String, Error>> {
        let table = table.clone();
        async move {
            self.with(move |c| {
                let sql = format!(
                    "SELECT sql FROM {}.sqlite_master WHERE tbl_name = ?1 AND sql IS NOT NULL \
                     ORDER BY type = 'index', name",
                    Dialect::Sqlite.ident(&schema_of(&table))
                );
                let mut stmt = c.prepare(&sql).map_err(error::map)?;
                let parts: Vec<String> = stmt
                    .query_map([table.name.as_str()], |r| r.get::<_, String>(0))
                    .map_err(error::map)?
                    .filter_map(Result::ok)
                    .map(|s| format!("{s};"))
                    .collect();
                if parts.is_empty() {
                    return Err(Error::not_found(table.to_string()));
                }
                Ok(parts.join("\n"))
            })
            .await
        }
        .boxed()
    }
}
