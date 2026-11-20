//! SQL dialect helpers: identifier quoting, literals, row `SELECT` builders, `RowEdit` to SQL,
//! chart `GROUP BY` builders and safe-mode statement checks.

mod dialect;
pub mod rows;

pub use dialect::Dialect;
