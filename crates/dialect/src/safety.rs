//! What a SQL script does, for safe mode: which statements write, and which `UPDATE` or
//! `DELETE` has no `WHERE`.

use sqlparser::ast::{SetExpr, Statement};
use sqlparser::dialect::{
    ClickHouseDialect, Dialect as ParserDialect, GenericDialect, MySqlDialect, PostgreSqlDialect,
    SQLiteDialect,
};
use sqlparser::parser::Parser;

use crate::Dialect;

/// One statement's effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementSafety {
    /// May change data, schema or session state. Unparsable statements count as writes.
    pub writes: bool,
    /// An `UPDATE` or `DELETE` without `WHERE`: touches every row.
    pub unbounded: bool,
}

fn parser_dialect(dialect: Option<Dialect>) -> Box<dyn ParserDialect> {
    match dialect {
        Some(Dialect::Postgres) => Box::new(PostgreSqlDialect {}),
        Some(Dialect::MySql) => Box::new(MySqlDialect {}),
        Some(Dialect::Sqlite) => Box::new(SQLiteDialect {}),
        Some(Dialect::ClickHouse) => Box::new(ClickHouseDialect {}),
        None => Box::new(GenericDialect {}),
    }
}

/// Only plain reads pass: queries without `INTO`, `EXPLAIN` of a read, `SHOW`, `DESCRIBE`,
/// `USE`. Everything else, including `SET` (which could turn the server's read-only mode off)
/// and `EXPLAIN ANALYZE` of a write (which runs it), writes.
fn reads_only(statement: &Statement) -> bool {
    match statement {
        Statement::Query(query) => !matches!(&*query.body, SetExpr::Select(s) if s.into.is_some()),
        Statement::Explain { statement, .. } => reads_only(statement),
        Statement::ExplainTable { .. }
        | Statement::ShowCatalogs { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowCollation { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowObjects(_)
        | Statement::ShowProcessList { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowTables { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowViews { .. }
        | Statement::Use(_) => true,
        _ => false,
    }
}

/// Effect of every statement in `sql` (generic SQL when the dialect is unknown). A script
/// that does not parse is one write.
#[must_use]
pub fn analyze(dialect: Option<Dialect>, sql: &str) -> Vec<StatementSafety> {
    match Parser::parse_sql(&*parser_dialect(dialect), sql) {
        Ok(statements) => statements
            .iter()
            .map(|s| StatementSafety {
                writes: !reads_only(s),
                unbounded: match s {
                    Statement::Update(u) => u.selection.is_none(),
                    Statement::Delete(d) => d.selection.is_none(),
                    _ => false,
                },
            })
            .collect(),
        Err(_) => vec![StatementSafety {
            writes: true,
            unbounded: false,
        }],
    }
}

/// True if any statement may write.
#[must_use]
pub fn writes(dialect: Option<Dialect>, sql: &str) -> bool {
    analyze(dialect, sql).iter().any(|s| s.writes)
}

/// True if any statement is an `UPDATE` or `DELETE` without `WHERE`.
#[must_use]
pub fn unbounded(dialect: Option<Dialect>, sql: &str) -> bool {
    analyze(dialect, sql).iter().any(|s| s.unbounded)
}

#[cfg(test)]
mod tests {
    use super::{unbounded, writes};
    use crate::Dialect;

    #[test]
    fn classifies_reads_and_writes() {
        let pg = Some(Dialect::Postgres);
        for read in [
            "SELECT 1",
            "select * from orders where id = 1",
            "EXPLAIN SELECT 1",
            "SHOW search_path",
            "WITH t AS (SELECT 1) SELECT * FROM t",
        ] {
            assert!(!writes(pg, read), "{read}");
        }
        for write in [
            "UPDATE orders SET status = 'paid' WHERE id = 1",
            "DELETE FROM orders",
            "INSERT INTO t VALUES (1)",
            "DROP TABLE t",
            "SET default_transaction_read_only = off",
            "BEGIN READ WRITE",
            "SELECT * INTO copy FROM orders",
            "EXPLAIN ANALYZE DELETE FROM orders",
            "SELECT 1; DELETE FROM t",
            "this is not sql",
        ] {
            assert!(writes(pg, write), "{write}");
        }
        assert!(unbounded(pg, "UPDATE orders SET status = 'x'"));
        assert!(unbounded(Some(Dialect::MySql), "DELETE FROM orders"));
        assert!(!unbounded(pg, "DELETE FROM orders WHERE id = 1"));
        assert!(!writes(
            Some(Dialect::ClickHouse),
            "SELECT count() FROM events"
        ));
    }
}
