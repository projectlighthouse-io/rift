//! Row edits.

use serde::{Deserialize, Serialize};

use crate::{ColumnName, TableRef, Value};

/// A column paired with a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnValue {
    /// Column.
    pub column: ColumnName,
    /// Value.
    pub value: Value,
}

impl ColumnValue {
    /// Pairs a column with a value.
    #[must_use]
    pub fn new(column: impl Into<ColumnName>, value: Value) -> Self {
        Self {
            column: column.into(),
            value,
        }
    }
}

/// What a [`RowEdit`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RowEditKind {
    /// Change some columns of an existing row.
    Update {
        /// New values.
        changes: Vec<ColumnValue>,
    },
    /// Insert a new row. `primary_key` may be empty when the database generates it.
    Insert {
        /// Values to insert.
        values: Vec<ColumnValue>,
    },
    /// Delete the row.
    Delete,
}

/// One pending edit to a row, identified by primary key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowEdit {
    /// Target table.
    pub table: TableRef,
    /// Primary key of the row.
    pub primary_key: Vec<ColumnValue>,
    /// The edit.
    pub kind: RowEditKind,
}
