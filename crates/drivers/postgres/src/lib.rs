//! Rift driver for PostgreSQL, built on `sqlx`. Everything is private except `factory()`.
//!
//! Results use the text protocol (`raw_sql`) so any type, including ones Rift does not model,
//! arrives as readable text. Queries are cancelled with `pg_cancel_backend` from a side
//! connection.

mod catalog;
mod connection;
mod error;
mod factory;
mod query;
mod values;

use traits::DriverFactory;

/// The PostgreSQL driver.
#[must_use]
pub fn factory() -> Box<dyn DriverFactory> {
    Box::new(factory::PgFactory::new())
}
