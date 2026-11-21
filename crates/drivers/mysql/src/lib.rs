//! Rift driver for MySQL, built on `sqlx`. Everything is private except `factory()`.
//!
//! Results use the text protocol (`raw_sql`); queries are cancelled with `KILL QUERY` from a
//! side connection. Structure changes cannot be rolled back on MySQL, which the driver reports
//! through `EditSchema::transactional_ddl`.

mod analysis;
mod backup;
mod catalog;
mod connection;
mod edit;
mod error;
mod factory;
mod query;
mod values;

use traits::DriverFactory;

/// The MySQL driver.
#[must_use]
pub fn factory() -> Box<dyn DriverFactory> {
    Box::new(factory::MySqlFactory::new())
}
