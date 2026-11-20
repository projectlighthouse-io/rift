//! Result and table columns.

use serde::{Deserialize, Serialize};

use crate::{ColumnName, TableRef};

/// A column in a result set or table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    /// Column name.
    pub name: ColumnName,
    /// Database type name as shown in the grid header, e.g. `int8`.
    pub type_name: String,
    /// Whether the column accepts `NULL`. `None` when unknown (query results).
    pub nullable: Option<bool>,
    /// Part of the primary key.
    pub primary_key: bool,
    /// Foreign key target, if any.
    pub references: Option<ColumnRef>,
}

impl Column {
    /// A result column with only a name and type.
    #[must_use]
    pub fn new(name: impl Into<ColumnName>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            nullable: None,
            primary_key: false,
            references: None,
        }
    }
}

/// Points at a column in another table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColumnRef {
    /// Referenced table.
    pub table: TableRef,
    /// Referenced column.
    pub column: ColumnName,
}
