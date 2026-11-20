//! Tables and other database objects.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{SchemaName, TableName};

/// A fully qualified table or view.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TableRef {
    /// Schema (Postgres, ClickHouse database, MySQL database). `None` for SQLite `main`.
    pub schema: Option<SchemaName>,
    /// Table name.
    pub name: TableName,
}

impl TableRef {
    /// A table in a schema.
    #[must_use]
    pub fn new(schema: Option<SchemaName>, name: impl Into<TableName>) -> Self {
        Self {
            schema,
            name: name.into(),
        }
    }
}

impl fmt::Display for TableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.schema {
            Some(schema) => write!(f, "{schema}.{}", self.name),
            None => write!(f, "{}", self.name),
        }
    }
}

/// Kind of object listed in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ObjectKind {
    /// Base table.
    Table,
    /// View.
    View,
    /// Materialized view.
    MaterializedView,
    /// Function or procedure.
    Function,
    /// Enum type.
    Enum,
}

/// An object in a schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbObject {
    /// The object.
    pub table: TableRef,
    /// What it is.
    pub kind: ObjectKind,
    /// Estimated row count for tables, if cheap to get.
    pub row_estimate: Option<u64>,
}
