//! Rift driver for Redis, built on `redis` with a tokio `ConnectionManager`. Everything is
//! private except `factory()`.
//!
//! Keys are listed with `SCAN` (never `KEYS`) and typed with a pipelined `TYPE`. Values are read
//! and written whole, in `MULTI`/`EXEC`, keeping the key's TTL.

mod connection;
mod factory;
mod kv;
mod memory;

use traits::DriverFactory;

/// The Redis driver.
#[must_use]
pub fn factory() -> Box<dyn DriverFactory> {
    Box::new(factory::RedisFactory::new())
}
