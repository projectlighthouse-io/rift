//! Browse and Describe from `system.*` tables.

use dialect::{Dialect, rows};
use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{Browse, Describe};
use types::{
    Column, ColumnDef, DbObject, Error, IndexDef, ObjectKind, Page, Row, RowsRequest, SchemaName,
    TableDescription, TableRef, Value,
};

use crate::connection::ClickHouseConnection;
use crate::values::base_type;

const SYSTEM: &str = "('system', 'INFORMATION_SCHEMA', 'information_schema')";

fn db(table: &TableRef) -> String {
    table
        .schema
        .as_ref()
        .map_or_else(|| "default".to_owned(), ToString::to_string)
}

fn s(v: Option<&Value>) -> String {
    v.map(ToString::to_string).unwrap_or_default()
}

fn n(v: Option<&Value>) -> Option<u64> {
    v.and_then(Value::as_f64).map(|f| f as u64)
}

impl ClickHouseConnection {
    async fn column_defs(&self, table: &TableRef) -> Result<Vec<ColumnDef>, Error> {
        let d = Dialect::ClickHouse;
        let sql = format!(
            "SELECT name, type, default_kind, default_expression, comment, is_in_primary_key \
             FROM system.columns WHERE database = {} AND table = {} ORDER BY position",
            d.string(&db(table)),
            d.string(table.name.as_str())
        );
        let rows = self.client.values(&sql).await?;
        if rows.is_empty() {
            return Err(Error::not_found(table.to_string()));
        }
        Ok(rows
            .iter()
            .map(|r| {
                let type_name = s(r.first().and(r.get(1)));
                let default = s(r.get(3));
                let comment = s(r.get(4));
                ColumnDef {
                    name: s(r.first()).into(),
                    nullable: type_name.starts_with("Nullable("),
                    type_name,
                    default: (!default.is_empty()).then_some(default),
                    primary_key: r.get(5).and_then(Value::as_f64).is_some_and(|v| v > 0.0),
                    references: None,
                    comment: (!comment.is_empty()).then_some(comment),
                }
            })
            .collect())
    }
}

impl Browse for ClickHouseConnection {
    fn schemas(&self) -> BoxFuture<'_, Result<Vec<SchemaName>, Error>> {
        async move {
            let sql = format!(
                "SELECT name FROM system.databases WHERE name NOT IN {SYSTEM} ORDER BY name"
            );
            Ok(self
                .client
                .values(&sql)
                .await?
                .into_iter()
                .map(|r| SchemaName::new(s(r.first())))
                .collect())
        }
        .boxed()
    }

    fn default_schema(&self) -> BoxFuture<'_, Result<Option<SchemaName>, Error>> {
        async move {
            let rows = self.client.values("SELECT currentDatabase()").await?;
            Ok(rows
                .into_iter()
                .next()
                .map(|r| SchemaName::new(s(r.first()))))
        }
        .boxed()
    }

    fn objects<'a>(
        &'a self,
        schema: &'a SchemaName,
    ) -> BoxFuture<'a, Result<Vec<DbObject>, Error>> {
        async move {
            let sql = format!(
                "SELECT name, engine, total_rows FROM system.tables WHERE database = {} \
                 AND NOT startsWith(name, '.inner') ORDER BY name",
                Dialect::ClickHouse.string(schema.as_str())
            );
            Ok(self
                .client
                .values(&sql)
                .await?
                .iter()
                .map(|r| DbObject {
                    table: TableRef::new(Some(schema.clone()), s(r.first())),
                    kind: match s(r.get(1)).as_str() {
                        "View" => ObjectKind::View,
                        "MaterializedView" => ObjectKind::MaterializedView,
                        _ => ObjectKind::Table,
                    },
                    row_estimate: n(r.get(2)),
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
            let defs = self.column_defs(table).await.unwrap_or_default();
            let sql = rows::select_rows(Dialect::ClickHouse, table, request);
            let response = self.client.run(&sql, None).await?;
            let columns = response
                .names
                .iter()
                .zip(&response.types)
                .map(|(name, t)| {
                    let def = defs.iter().find(|d| d.name.as_str() == name);
                    Column {
                        name: name.as_str().into(),
                        type_name: base_type(t).to_owned(),
                        nullable: def.map(|d| d.nullable),
                        primary_key: def.is_some_and(|d| d.primary_key),
                        references: None,
                    }
                })
                .collect();
            let rows: Result<Vec<Vec<Value>>, Error> =
                futures::StreamExt::collect::<Vec<_>>(response.rows)
                    .await
                    .into_iter()
                    .collect();
            Ok(Page {
                columns,
                rows: rows?.into_iter().map(|values| Row { values }).collect(),
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
            let rows = self
                .client
                .values(&rows::count_rows(Dialect::ClickHouse, table, request))
                .await?;
            Ok(rows.first().and_then(|r| n(r.first())).unwrap_or(0))
        }
        .boxed()
    }
}

impl Describe for ClickHouseConnection {
    fn describe<'a>(
        &'a self,
        table: &'a TableRef,
    ) -> BoxFuture<'a, Result<TableDescription, Error>> {
        async move {
            let d = Dialect::ClickHouse;
            let columns = self.column_defs(table).await?;
            let (dbs, name) = (d.string(&db(table)), d.string(table.name.as_str()));
            let meta = self
                .client
                .values(&format!(
                    "SELECT primary_key, total_rows, total_bytes FROM system.tables WHERE database = {dbs} AND name = {name}"
                ))
                .await?;
            let meta = meta.into_iter().next().unwrap_or_default();
            let mut indexes = Vec::new();
            let primary = s(meta.first());
            if !primary.is_empty() {
                indexes.push(IndexDef {
                    name: "PRIMARY KEY".into(),
                    columns: primary.split(", ").map(str::to_owned).collect(),
                    method: Some("sparse".into()),
                    unique: false,
                    primary: true,
                    size_bytes: None,
                });
            }
            for r in self
                .client
                .values(&format!(
                    "SELECT name, expr, type FROM system.data_skipping_indices WHERE database = {dbs} AND table = {name}"
                ))
                .await?
            {
                indexes.push(IndexDef {
                    name: s(r.first()).into(),
                    columns: vec![s(r.get(1))],
                    method: Some(s(r.get(2))),
                    unique: false,
                    primary: false,
                    size_bytes: None,
                });
            }
            Ok(TableDescription {
                table: table.clone(),
                columns,
                indexes,
                foreign_keys: Vec::new(),
                row_estimate: n(meta.get(1)),
                size_bytes: n(meta.get(2)),
            })
        }
        .boxed()
    }

    fn ddl<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<String, Error>> {
        async move {
            let rows = self
                .client
                .values(&format!(
                    "SHOW CREATE TABLE {}",
                    Dialect::ClickHouse.table(table)
                ))
                .await?;
            rows.into_iter()
                .next()
                .and_then(|r| r.into_iter().next())
                .map(|v| format!("{v};"))
                .ok_or_else(|| Error::not_found(table.to_string()))
        }
        .boxed()
    }
}
