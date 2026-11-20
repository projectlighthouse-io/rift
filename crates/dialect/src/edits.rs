//! `RowEdit` to SQL.

use types::{ColumnValue, Error, RowEdit, RowEditKind};

use crate::Dialect;

fn where_key(dialect: Dialect, key: &[ColumnValue]) -> Result<String, Error> {
    if key.is_empty() {
        return Err(Error::ReadOnly {
            reason: "table has no primary key".into(),
        });
    }
    let parts: Vec<String> = key
        .iter()
        .map(|kv| {
            let column = dialect.ident(kv.column.as_str());
            if kv.value.is_null() {
                format!("{column} IS NULL")
            } else {
                format!("{column} = {}", dialect.literal(&kv.value))
            }
        })
        .collect();
    Ok(parts.join(" AND "))
}

/// The statement for one edit. Updates and deletes target exactly one row by primary key.
///
/// # Errors
/// [`Error::ReadOnly`] when an update or delete has no key; [`Error::Invalid`] for an empty
/// update.
pub fn edit_sql(dialect: Dialect, edit: &RowEdit) -> Result<String, Error> {
    let table = dialect.table(&edit.table);
    match &edit.kind {
        RowEditKind::Update { changes } => {
            if changes.is_empty() {
                return Err(Error::invalid("update without changes"));
            }
            let set: Vec<String> = changes
                .iter()
                .map(|cv| {
                    format!(
                        "{} = {}",
                        dialect.ident(cv.column.as_str()),
                        dialect.literal(&cv.value)
                    )
                })
                .collect();
            Ok(format!(
                "UPDATE {table} SET {} WHERE {}",
                set.join(", "),
                where_key(dialect, &edit.primary_key)?
            ))
        }
        RowEditKind::Insert { values } if values.is_empty() => Ok(match dialect {
            Dialect::MySql => format!("INSERT INTO {table} () VALUES ()"),
            _ => format!("INSERT INTO {table} DEFAULT VALUES"),
        }),
        RowEditKind::Insert { values } => {
            let columns: Vec<String> = values
                .iter()
                .map(|cv| dialect.ident(cv.column.as_str()))
                .collect();
            let literals: Vec<String> =
                values.iter().map(|cv| dialect.literal(&cv.value)).collect();
            Ok(format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                columns.join(", "),
                literals.join(", ")
            ))
        }
        RowEditKind::Delete => Ok(format!(
            "DELETE FROM {table} WHERE {}",
            where_key(dialect, &edit.primary_key)?
        )),
        _ => Err(Error::unsupported("row edit kind")),
    }
}

/// Whether the statement for `edit` must change exactly one row.
#[must_use]
pub fn expects_one_row(edit: &RowEdit) -> bool {
    matches!(edit.kind, RowEditKind::Update { .. } | RowEditKind::Delete)
}

#[cfg(test)]
mod tests {
    use types::{ColumnValue, RowEdit, RowEditKind, SchemaName, TableRef, Value};

    use super::edit_sql;
    use crate::Dialect;

    fn orders() -> TableRef {
        TableRef::new(Some(SchemaName::new("public")), "orders")
    }

    fn edits() -> Vec<RowEdit> {
        vec![
            RowEdit {
                table: orders(),
                primary_key: vec![ColumnValue::new("id", Value::Int(1042))],
                kind: RowEditKind::Update {
                    changes: vec![
                        ColumnValue::new("status", Value::Text("shipped".into())),
                        ColumnValue::new("notes", Value::Text("it's fragile".into())),
                    ],
                },
            },
            RowEdit {
                table: orders(),
                primary_key: Vec::new(),
                kind: RowEditKind::Insert {
                    values: vec![
                        ColumnValue::new("customer_id", Value::Int(130)),
                        ColumnValue::new("total", Value::Text("45.00".into())),
                        ColumnValue::new("shipped_at", Value::Null),
                    ],
                },
            },
            RowEdit {
                table: orders(),
                primary_key: vec![ColumnValue::new("id", Value::Int(1045))],
                kind: RowEditKind::Delete,
            },
            RowEdit {
                table: orders(),
                primary_key: Vec::new(),
                kind: RowEditKind::Insert { values: Vec::new() },
            },
        ]
    }

    fn render(dialect: Dialect) -> String {
        edits()
            .iter()
            .map(|e| edit_sql(dialect, e).expect("sql"))
            .collect::<Vec<_>>()
            .join(";\n")
    }

    #[test]
    fn postgres_edits() {
        insta::assert_snapshot!(render(Dialect::Postgres));
    }

    #[test]
    fn mysql_edits() {
        insta::assert_snapshot!(render(Dialect::MySql));
    }

    #[test]
    fn sqlite_edits() {
        insta::assert_snapshot!(render(Dialect::Sqlite));
    }

    #[test]
    fn keyless_update_is_read_only() {
        let mut edit = edits().remove(0);
        edit.primary_key.clear();
        assert!(matches!(
            edit_sql(Dialect::Postgres, &edit),
            Err(types::Error::ReadOnly { .. })
        ));
    }
}
