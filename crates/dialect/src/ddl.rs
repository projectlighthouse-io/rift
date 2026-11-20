//! `SchemaChange` to DDL.

use types::{Error, SchemaChange, TableRef};

use crate::Dialect;

/// Index column list: plain names are quoted as identifiers, expressions (`created_at DESC`,
/// `lower(email)`) are kept as typed.
fn index_columns(dialect: Dialect, columns: &[String]) -> String {
    columns
        .iter()
        .map(|c| {
            let mut parts = c.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some(name), dir, None)
                    if dir.is_none_or(|d| {
                        d.eq_ignore_ascii_case("asc") || d.eq_ignore_ascii_case("desc")
                    }) && !name.contains('(') =>
                {
                    match dir {
                        Some(d) => format!("{} {}", dialect.ident(name), d.to_uppercase()),
                        None => dialect.ident(name),
                    }
                }
                _ => c.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Statements for one structure change, in order.
///
/// # Errors
/// [`Error::Unsupported`] for changes the engine cannot do; [`Error::Invalid`] for empty names.
pub fn schema_change_sql(
    dialect: Dialect,
    table: &TableRef,
    change: &SchemaChange,
) -> Result<Vec<String>, Error> {
    let t = dialect.table(table);
    Ok(match change {
        SchemaChange::AddColumn { column } => {
            if column.name.as_str().is_empty() || column.type_name.trim().is_empty() {
                return Err(Error::invalid("a new column needs a name and a type"));
            }
            let mut sql = format!(
                "ALTER TABLE {t} ADD COLUMN {} {}",
                dialect.ident(column.name.as_str()),
                column.type_name.trim()
            );
            if let Some(default) = column.default.as_deref().filter(|d| !d.trim().is_empty()) {
                sql.push_str(&format!(" DEFAULT {}", default.trim()));
            }
            if !column.nullable {
                sql.push_str(" NOT NULL");
            }
            let mut statements = vec![sql];
            if let Some(comment) = column.comment.as_deref().filter(|c| !c.is_empty()) {
                match dialect {
                    Dialect::Postgres => statements.push(format!(
                        "COMMENT ON COLUMN {t}.{} IS {}",
                        dialect.ident(column.name.as_str()),
                        dialect.string(comment)
                    )),
                    Dialect::MySql | Dialect::ClickHouse => {
                        statements[0].push_str(&format!(" COMMENT {}", dialect.string(comment)));
                    }
                    _ => {}
                }
            }
            statements
        }
        SchemaChange::DropColumn { name } => {
            vec![format!(
                "ALTER TABLE {t} DROP COLUMN {}",
                dialect.ident(name.as_str())
            )]
        }
        SchemaChange::RenameColumn { from, to } => vec![format!(
            "ALTER TABLE {t} RENAME COLUMN {} TO {}",
            dialect.ident(from.as_str()),
            dialect.ident(to.as_str())
        )],
        SchemaChange::AddIndex { index } => {
            if index.name.as_str().is_empty() || index.columns.is_empty() {
                return Err(Error::invalid("a new index needs a name and columns"));
            }
            let unique = if index.unique { "UNIQUE " } else { "" };
            let name = dialect.ident(index.name.as_str());
            let columns = index_columns(dialect, &index.columns);
            match dialect {
                Dialect::ClickHouse => vec![format!(
                    "ALTER TABLE {t} ADD INDEX {name} ({columns}) TYPE {} GRANULARITY 4",
                    index.method.as_deref().unwrap_or("minmax")
                )],
                Dialect::Postgres => {
                    let method = index
                        .method
                        .as_deref()
                        .map_or(String::new(), |m| format!(" USING {m}"));
                    vec![format!(
                        "CREATE {unique}INDEX {name} ON {t}{method} ({columns})"
                    )]
                }
                _ => vec![format!("CREATE {unique}INDEX {name} ON {t} ({columns})")],
            }
        }
        SchemaChange::DropIndex { name } => {
            let index = dialect.ident(name.as_str());
            match dialect {
                Dialect::Postgres => match &table.schema {
                    Some(schema) => vec![format!(
                        "DROP INDEX {}.{index}",
                        dialect.ident(schema.as_str())
                    )],
                    None => vec![format!("DROP INDEX {index}")],
                },
                Dialect::MySql => vec![format!("DROP INDEX {index} ON {t}")],
                Dialect::ClickHouse => vec![format!("ALTER TABLE {t} DROP INDEX {index}")],
                _ => vec![format!("DROP INDEX {index}")],
            }
        }
        SchemaChange::Truncate => match dialect {
            Dialect::Sqlite => vec![format!("DELETE FROM {t}")],
            _ => vec![format!("TRUNCATE TABLE {t}")],
        },
        _ => return Err(Error::unsupported("schema change")),
    })
}

/// Statements for a list of changes.
///
/// # Errors
/// The first change that cannot be expressed.
pub fn schema_changes_sql(
    dialect: Dialect,
    table: &TableRef,
    changes: &[SchemaChange],
) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    for change in changes {
        out.extend(schema_change_sql(dialect, table, change)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use types::{ColumnDef, IndexDef, SchemaChange, SchemaName, TableRef};

    use super::schema_changes_sql;
    use crate::Dialect;

    fn changes() -> Vec<SchemaChange> {
        vec![
            SchemaChange::AddColumn {
                column: ColumnDef {
                    name: "gift_message".into(),
                    type_name: "text".into(),
                    nullable: true,
                    default: None,
                    primary_key: false,
                    references: None,
                    comment: Some("Shown on the packing slip".into()),
                },
            },
            SchemaChange::AddColumn {
                column: ColumnDef {
                    name: "Priority".into(),
                    type_name: "int".into(),
                    nullable: false,
                    default: Some("0".into()),
                    primary_key: false,
                    references: None,
                    comment: None,
                },
            },
            SchemaChange::DropColumn {
                name: "notes".into(),
            },
            SchemaChange::RenameColumn {
                from: "total".into(),
                to: "order".into(),
            },
            SchemaChange::AddIndex {
                index: IndexDef {
                    name: "orders_status_created_idx".into(),
                    columns: vec!["status".into(), "created_at desc".into()],
                    method: None,
                    unique: false,
                    primary: false,
                    size_bytes: None,
                },
            },
            SchemaChange::DropIndex {
                name: "orders_customer_id_idx".into(),
            },
            SchemaChange::Truncate,
        ]
    }

    fn render(dialect: Dialect) -> String {
        let table = TableRef::new(Some(SchemaName::new("public")), "orders");
        schema_changes_sql(dialect, &table, &changes())
            .expect("ddl")
            .join(";\n")
    }

    #[test]
    fn postgres_ddl() {
        insta::assert_snapshot!(render(Dialect::Postgres));
    }

    #[test]
    fn mysql_ddl() {
        insta::assert_snapshot!(render(Dialect::MySql));
    }

    #[test]
    fn sqlite_ddl() {
        insta::assert_snapshot!(render(Dialect::Sqlite));
    }
}
