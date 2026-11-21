//! SQL dialects: identifier quoting and literals.

use types::{TableRef, Value};

/// SQL flavour of a driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dialect {
    /// PostgreSQL.
    Postgres,
    /// MySQL / MariaDB.
    MySql,
    /// SQLite.
    Sqlite,
    /// ClickHouse.
    ClickHouse,
}

impl Dialect {
    /// The dialect a driver speaks, by driver id; `None` for drivers without SQL.
    #[must_use]
    pub fn for_driver(id: &str) -> Option<Self> {
        match id {
            "postgres" => Some(Dialect::Postgres),
            "mysql" => Some(Dialect::MySql),
            "sqlite" => Some(Dialect::Sqlite),
            "clickhouse" => Some(Dialect::ClickHouse),
            _ => None,
        }
    }

    /// Quotes an identifier when needed: `"Name"` or `` `my col` `` for MySQL and ClickHouse.
    /// Lower-case names that are not keywords are left bare, so generated SQL reads naturally.
    #[must_use]
    pub fn ident(self, name: &str) -> String {
        if is_plain_identifier(name) {
            return name.to_owned();
        }
        match self {
            Dialect::MySql | Dialect::ClickHouse => format!("`{}`", name.replace('`', "``")),
            _ => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    /// Quotes a possibly schema-qualified table.
    #[must_use]
    pub fn table(self, table: &TableRef) -> String {
        match &table.schema {
            Some(schema) => format!(
                "{}.{}",
                self.ident(schema.as_str()),
                self.ident(table.name.as_str())
            ),
            None => self.ident(table.name.as_str()),
        }
    }

    /// Whether string literals treat backslash as an escape character.
    fn backslash_escapes(self) -> bool {
        matches!(self, Dialect::MySql | Dialect::ClickHouse)
    }

    /// Quotes a string literal.
    #[must_use]
    pub fn string(self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 2);
        out.push('\'');
        for c in text.chars() {
            match c {
                '\'' => out.push_str("''"),
                '\\' if self.backslash_escapes() => out.push_str("\\\\"),
                _ => out.push(c),
            }
        }
        out.push('\'');
        out
    }

    /// Renders a value as a SQL literal. Strings are untyped so the database coerces them to
    /// the column type (enums, timestamps, numerics), which avoids per-driver parameter binding.
    #[must_use]
    pub fn literal(self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".into(),
            Value::Bool(v) => match self {
                Dialect::MySql | Dialect::Sqlite => if *v { "1" } else { "0" }.into(),
                _ => if *v { "TRUE" } else { "FALSE" }.into(),
            },
            Value::Int(v) => v.to_string(),
            Value::UInt(v) => v.to_string(),
            Value::Float(v) if v.is_finite() => format!("{v:?}"),
            Value::Float(v) => self.string(&v.to_string()),
            Value::Decimal(v) => v.to_string(),
            Value::Bytes(bytes) => {
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                match self {
                    Dialect::Postgres => format!("'\\x{hex}'"),
                    Dialect::ClickHouse => format!("unhex('{hex}')"),
                    _ => format!("X'{hex}'"),
                }
            }
            Value::Array(items) => match self {
                Dialect::Postgres => self.string(&value.to_string()),
                _ => format!(
                    "[{}]",
                    items
                        .iter()
                        .map(|v| self.literal(v))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
            other => self.string(&other.to_string()),
        }
    }

    /// Placeholder-free `LIMIT`/`OFFSET` clause.
    #[must_use]
    pub fn limit(self, limit: u64, offset: u64) -> String {
        if offset == 0 {
            format!(" LIMIT {limit}")
        } else {
            format!(" LIMIT {limit} OFFSET {offset}")
        }
    }
}

/// Words that must be quoted as identifiers in at least one supported engine.
const KEYWORDS: &[&str] = &[
    "all",
    "alter",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "between",
    "both",
    "by",
    "case",
    "cast",
    "check",
    "collate",
    "column",
    "constraint",
    "create",
    "cross",
    "current_date",
    "current_time",
    "current_timestamp",
    "current_user",
    "database",
    "default",
    "delete",
    "desc",
    "distinct",
    "do",
    "drop",
    "else",
    "end",
    "except",
    "exists",
    "false",
    "fetch",
    "for",
    "foreign",
    "from",
    "full",
    "grant",
    "group",
    "having",
    "if",
    "in",
    "index",
    "inner",
    "insert",
    "intersect",
    "interval",
    "into",
    "is",
    "join",
    "key",
    "keys",
    "leading",
    "left",
    "like",
    "limit",
    "localtime",
    "localtimestamp",
    "match",
    "natural",
    "not",
    "null",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "outer",
    "over",
    "partition",
    "primary",
    "range",
    "references",
    "returning",
    "right",
    "row",
    "rows",
    "schema",
    "select",
    "session_user",
    "set",
    "some",
    "table",
    "then",
    "to",
    "trailing",
    "true",
    "union",
    "unique",
    "update",
    "user",
    "using",
    "values",
    "view",
    "when",
    "where",
    "window",
    "with",
];

/// `[a-z_][a-z0-9_]*` and not a keyword.
fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !KEYWORDS.contains(&name)
}
