//! Key-value access and expiry.

use std::time::Duration;

use futures::future::BoxFuture;
use types::{Error, KeyBytes, KeyPage, KeyScan, KvValue};

/// Scans, reads and writes keys.
///
/// Implemented by: Redis (`SCAN`, never `KEYS`), FoundationDB (range reads).
pub trait KeyValue: Send + Sync {
    /// One page of keys.
    fn scan<'a>(&'a self, scan: &'a KeyScan) -> BoxFuture<'a, Result<KeyPage, Error>>;
    /// Reads a whole value; `None` if the key does not exist.
    fn get<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<KvValue>, Error>>;
    /// Replaces a value. Keeps the key's TTL where the engine allows.
    fn set<'a>(&'a self, key: &'a KeyBytes, value: &'a KvValue)
    -> BoxFuture<'a, Result<(), Error>>;
    /// Deletes a key; returns whether it existed.
    fn delete<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<bool, Error>>;
    /// Renames a key. Fails if `to` exists.
    fn rename<'a>(
        &'a self,
        from: &'a KeyBytes,
        to: &'a KeyBytes,
    ) -> BoxFuture<'a, Result<(), Error>>;
    /// Approximate number of keys.
    fn key_count(&self) -> BoxFuture<'_, Result<u64, Error>>;
}

/// Per-key time to live.
///
/// Implemented by: Redis.
pub trait KeyExpiry: Send + Sync {
    /// Remaining TTL; `None` if the key never expires.
    fn ttl<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<Duration>, Error>>;
    /// Sets (`Some`) or removes (`None`) the TTL.
    fn set_ttl<'a>(
        &'a self,
        key: &'a KeyBytes,
        ttl: Option<Duration>,
    ) -> BoxFuture<'a, Result<(), Error>>;
}
