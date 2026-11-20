//! Pages of rows.

use serde::{Deserialize, Serialize};

use crate::{Column, ColumnName, Value};

/// One row of values, in column order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Cell values.
    pub values: Vec<Value>,
}

/// A page of rows with their columns.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Page {
    /// Columns.
    pub columns: Vec<Column>,
    /// Rows.
    pub rows: Vec<Row>,
}

impl Page {
    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when the page has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Rows per streamed page.
pub const PAGE_SIZE: usize = 500;

/// Comparison in a table filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum FilterOp {
    /// `=`
    Eq,
    /// `<>`
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    Lte,
    /// `>`
    Gt,
    /// `>=`
    Gte,
    /// Case-insensitive substring match.
    Contains,
    /// `IS NULL`
    IsNull,
    /// `IS NOT NULL`
    IsNotNull,
}

/// One filter condition on a column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// Column to test.
    pub column: ColumnName,
    /// Comparison.
    pub op: FilterOp,
    /// Right-hand value; ignored by `IsNull`/`IsNotNull`.
    pub value: Value,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SortDir {
    /// Ascending.
    Asc,
    /// Descending.
    Desc,
}

/// Sort on one column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    /// Column.
    pub column: ColumnName,
    /// Direction.
    pub dir: SortDir,
}

/// Request for a page of table rows.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RowsRequest {
    /// Conditions, combined with `AND`.
    pub filters: Vec<Filter>,
    /// Sort order.
    pub sort: Vec<Sort>,
    /// Rows to skip.
    pub offset: u64,
    /// Max rows.
    pub limit: u64,
}
