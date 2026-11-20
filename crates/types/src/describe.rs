//! Table structure.

use serde::{Deserialize, Serialize};

use crate::{ColumnName, ColumnRef, IndexName, TableRef};

/// Full column definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnDef {
    /// Name.
    pub name: ColumnName,
    /// Type, as written in DDL, e.g. `numeric(10,2)`.
    pub type_name: String,
    /// Accepts `NULL`.
    pub nullable: bool,
    /// Default expression.
    pub default: Option<String>,
    /// Part of the primary key.
    pub primary_key: bool,
    /// Foreign key target.
    pub references: Option<ColumnRef>,
    /// Column comment.
    pub comment: Option<String>,
}

/// An index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexDef {
    /// Name.
    pub name: IndexName,
    /// Column expressions, e.g. `created_at DESC`.
    pub columns: Vec<String>,
    /// Access method, e.g. `btree`.
    pub method: Option<String>,
    /// Unique index.
    pub unique: bool,
    /// Backs the primary key.
    pub primary: bool,
    /// Size on disk.
    pub size_bytes: Option<u64>,
}

/// A foreign key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeignKeyDef {
    /// Constraint name.
    pub name: String,
    /// Local columns.
    pub columns: Vec<ColumnName>,
    /// Referenced table.
    pub ref_table: TableRef,
    /// Referenced columns.
    pub ref_columns: Vec<ColumnName>,
    /// `ON DELETE` action.
    pub on_delete: String,
    /// `ON UPDATE` action.
    pub on_update: String,
}

/// Structure of a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableDescription {
    /// The table.
    pub table: TableRef,
    /// Columns in ordinal order.
    pub columns: Vec<ColumnDef>,
    /// Indexes.
    pub indexes: Vec<IndexDef>,
    /// Foreign keys.
    pub foreign_keys: Vec<ForeignKeyDef>,
    /// Estimated rows.
    pub row_estimate: Option<u64>,
    /// Total size on disk.
    pub size_bytes: Option<u64>,
}

impl TableDescription {
    /// Primary key columns, in order.
    #[must_use]
    pub fn primary_key(&self) -> Vec<&ColumnName> {
        self.columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| &c.name)
            .collect()
    }
}

/// A pending change to a table's structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SchemaChange {
    /// Add a column.
    AddColumn {
        /// Definition; `primary_key`/`references` are ignored.
        column: ColumnDef,
    },
    /// Drop a column.
    DropColumn {
        /// Column.
        name: ColumnName,
    },
    /// Rename a column.
    RenameColumn {
        /// Current name.
        from: ColumnName,
        /// New name.
        to: ColumnName,
    },
    /// Create an index.
    AddIndex {
        /// Definition.
        index: IndexDef,
    },
    /// Drop an index.
    DropIndex {
        /// Name.
        name: IndexName,
    },
    /// Delete all rows.
    Truncate,
}
