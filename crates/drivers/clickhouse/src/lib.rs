//! Rift driver for ClickHouse over its HTTP interface. Everything is private except
//! `factory()`.
//!
//! Results stream as `JSONCompactEachRowWithNamesAndTypes` (names, then types, then one JSON
//! array per row), so any column type maps to a `Value` at runtime. Queries carry a
//! `query_id` and are cancelled with `KILL QUERY`. ClickHouse is read-mostly here: no
//! `EditRows` and no `Transactions`.

mod catalog;
mod client;
mod connection;
mod factory;
mod values;

use traits::DriverFactory;

/// The ClickHouse driver.
#[must_use]
pub fn factory() -> Box<dyn DriverFactory> {
    Box::new(factory::ClickHouseFactory::new())
}
