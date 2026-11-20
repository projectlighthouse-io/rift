//! Rift driver for FoundationDB. Everything is private except `factory()`.
//!
//! The client library is loaded at runtime through `fdb-ffi`, so Rift runs without it and
//! connecting explains how to install it. Keys show as tuples through the tuple layer; the
//! sidebar lists the directory layer.

mod connection;
mod directory;
mod factory;
#[cfg(test)]
mod live_tests;
mod tuple;

use traits::DriverFactory;

/// The FoundationDB driver.
#[must_use]
pub fn factory() -> Box<dyn DriverFactory> {
    Box::new(factory::FdbFactory::new())
}
