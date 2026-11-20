//! SQL dialect helpers: identifier quoting, literals, row `SELECT` builders, `RowEdit` to SQL,
//! chart `GROUP BY` builders and safe-mode statement checks.

pub mod chart;
pub mod ddl;
mod dialect;
pub mod edits;
pub mod rows;
pub mod split;

pub use dialect::Dialect;
