//! `SELECT` and `COUNT` builders for table browsing.

use types::{Filter, FilterOp, RowsRequest, SortDir, TableRef, Value};

use crate::Dialect;

fn condition(dialect: Dialect, filter: &Filter) -> String {
    let column = dialect.ident(filter.column.as_str());
    let value = dialect.literal(&filter.value);
    match filter.op {
        FilterOp::Eq => format!("{column} = {value}"),
        FilterOp::NotEq => format!("{column} <> {value}"),
        FilterOp::Lt => format!("{column} < {value}"),
        FilterOp::Lte => format!("{column} <= {value}"),
        FilterOp::Gt => format!("{column} > {value}"),
        FilterOp::Gte => format!("{column} >= {value}"),
        FilterOp::IsNull => format!("{column} IS NULL"),
        FilterOp::IsNotNull => format!("{column} IS NOT NULL"),
        FilterOp::Contains => {
            let pattern = dialect.literal(&Value::Text(format!("%{}%", filter.value)));
            match dialect {
                Dialect::Postgres => format!("{column}::text ILIKE {pattern}"),
                Dialect::ClickHouse => format!("toString({column}) ILIKE {pattern}"),
                Dialect::MySql => format!("LOWER(CAST({column} AS CHAR)) LIKE LOWER({pattern})"),
                _ => format!("LOWER(CAST({column} AS TEXT)) LIKE LOWER({pattern})"),
            }
        }
        _ => "TRUE".into(),
    }
}

/// `WHERE ...` for the request's filters, or an empty string.
#[must_use]
pub fn where_clause(dialect: Dialect, filters: &[Filter]) -> String {
    if filters.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = filters.iter().map(|f| condition(dialect, f)).collect();
    format!(" WHERE {}", parts.join(" AND "))
}

/// `SELECT * FROM table WHERE ... ORDER BY ... LIMIT ... OFFSET ...`.
#[must_use]
pub fn select_rows(dialect: Dialect, table: &TableRef, request: &RowsRequest) -> String {
    let mut sql = format!("SELECT * FROM {}", dialect.table(table));
    sql.push_str(&where_clause(dialect, &request.filters));
    if !request.sort.is_empty() {
        let parts: Vec<String> = request
            .sort
            .iter()
            .map(|s| {
                let dir = if s.dir == SortDir::Desc {
                    "DESC"
                } else {
                    "ASC"
                };
                format!("{} {dir}", dialect.ident(s.column.as_str()))
            })
            .collect();
        sql.push_str(&format!(" ORDER BY {}", parts.join(", ")));
    }
    if request.limit > 0 {
        sql.push_str(&dialect.limit(request.limit, request.offset));
    }
    sql
}

/// `SELECT count(*) FROM table WHERE ...`.
#[must_use]
pub fn count_rows(dialect: Dialect, table: &TableRef, request: &RowsRequest) -> String {
    format!(
        "SELECT count(*) FROM {}{}",
        dialect.table(table),
        where_clause(dialect, &request.filters)
    )
}

#[cfg(test)]
mod tests {
    use types::{Filter, FilterOp, RowsRequest, SchemaName, Sort, SortDir, TableRef, Value};

    use crate::Dialect;

    #[test]
    fn select_with_filters_sort_and_paging() {
        let table = TableRef::new(Some(SchemaName::new("public")), "orders");
        let request = RowsRequest {
            filters: vec![
                Filter {
                    column: "status".into(),
                    op: FilterOp::NotEq,
                    value: Value::Text("o'k".into()),
                },
                Filter {
                    column: "notes".into(),
                    op: FilterOp::IsNull,
                    value: Value::Null,
                },
            ],
            sort: vec![Sort {
                column: "id".into(),
                dir: SortDir::Desc,
            }],
            offset: 50,
            limit: 50,
        };
        assert_eq!(
            super::select_rows(Dialect::Postgres, &table, &request),
            "SELECT * FROM \"public\".\"orders\" WHERE \"status\" <> 'o''k' AND \"notes\" IS NULL \
             ORDER BY \"id\" DESC LIMIT 50 OFFSET 50"
        );
        assert_eq!(
            super::count_rows(Dialect::MySql, &table, &request),
            "SELECT count(*) FROM `public`.`orders` WHERE `status` <> 'o''k' AND `notes` IS NULL"
        );
    }

    #[test]
    fn literals_escape_per_dialect() {
        let v = Value::Text("a\\b'".into());
        assert_eq!(Dialect::Postgres.literal(&v), "'a\\b'''");
        assert_eq!(Dialect::MySql.literal(&v), "'a\\\\b'''");
        assert_eq!(
            Dialect::Postgres.literal(&Value::Bytes(vec![0xab])),
            "'\\xab'"
        );
        assert_eq!(Dialect::Sqlite.literal(&Value::Bool(true)), "1");
    }
}
