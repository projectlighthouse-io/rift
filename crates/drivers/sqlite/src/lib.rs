//! Rift driver for SQLite, built on `rusqlite`. Everything is private except `factory()`.
//!
//! Every call opens the database file on a blocking thread, so long reads never hold a lock
//! the UI needs. Queries are cancelled with `sqlite3_interrupt`.

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

/// The SQLite driver.
#[must_use]
pub fn factory() -> Box<dyn DriverFactory> {
    Box::new(factory::SqliteFactory::new())
}
