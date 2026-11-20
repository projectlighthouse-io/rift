//! The FoundationDB directory layer.

use futures::future::BoxFuture;
use types::{Error, KeyBytes, KeyDirectory};

/// Lists named key prefixes and counts the keys under them.
///
/// Implemented by: FoundationDB (directory layer).
pub trait KeyDirectories: Send + Sync {
    /// Every directory, parents before children.
    fn directories(&self) -> BoxFuture<'_, Result<Vec<KeyDirectory>, Error>>;
    /// Keys under `prefix`, counted up to `cap`.
    fn count<'a>(&'a self, prefix: &'a KeyBytes, cap: u64) -> BoxFuture<'a, Result<u64, Error>>;
}
