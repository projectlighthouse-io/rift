//! The FoundationDB C client (`libfdb_c`), loaded at runtime so Rift builds and runs on
//! machines without it. All `unsafe` lives in [`ffi`]; this module is the safe, blocking API.
//!
//! Every call that waits on the cluster blocks the calling thread; run them on a blocking pool.

mod ffi;

use std::fmt;

pub use ffi::{Database, Transaction};

/// API version Rift speaks: supported by every 7.x client library.
pub const API_VERSION: i32 = 710;

/// An error from the client library (or from loading it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdbError {
    /// FoundationDB error code; `0` when the library could not be loaded.
    pub code: i32,
    /// Human-readable message.
    pub message: String,
}

impl fmt::Display for FdbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.code == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "{} ({})", self.message, self.code)
        }
    }
}

impl std::error::Error for FdbError {}

/// One end of a range read: the first key `>=` (or `>`) `key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySelector {
    /// Anchor key.
    pub key: Vec<u8>,
    /// Skip `key` itself.
    pub after: bool,
}

impl KeySelector {
    /// First key `>= key`.
    #[must_use]
    pub fn at_or_after(key: Vec<u8>) -> Self {
        Self { key, after: false }
    }

    /// First key `> key`.
    #[must_use]
    pub fn after(key: Vec<u8>) -> Self {
        Self { key, after: true }
    }
}

/// A key and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValue {
    /// Key bytes.
    pub key: Vec<u8>,
    /// Value bytes.
    pub value: Vec<u8>,
}

/// Loads `libfdb_c` once, selects [`API_VERSION`] and starts the network thread.
///
/// Looks at `RIFT_FDB_LIB`, then the standard install locations.
///
/// # Errors
/// When the library is missing or the API version is not supported.
pub fn init() -> Result<(), FdbError> {
    ffi::api_loaded()
}

/// Opens a database from a cluster file (`None` for the default one).
///
/// # Errors
/// When the library is missing or the cluster file is invalid.
pub fn open(cluster_file: Option<&str>) -> Result<Database, FdbError> {
    ffi::open(cluster_file)
}
