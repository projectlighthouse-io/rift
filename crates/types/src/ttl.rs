//! Table-level TTL rules (ClickHouse).

use serde::{Deserialize, Serialize};

use crate::ColumnName;

/// A TTL rule on a table or column, e.g. `ts + INTERVAL 30 DAY`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtlRule {
    /// Column the rule applies to; `None` for a whole-row (table) TTL.
    pub column: Option<ColumnName>,
    /// TTL expression as the database writes it.
    pub expression: String,
}
