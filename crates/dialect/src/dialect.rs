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
    /// Quotes an identifier: `"name"` or `` `name` `` for MySQL and ClickHouse.
    #[must_use]
    pub fn ident(self, name: &str) -> String {
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
