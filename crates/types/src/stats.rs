//! Column and memory statistics.

use serde::{Deserialize, Serialize};

use crate::{KeyKind, Value};

/// A value and how often it occurs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueCount {
    /// Value.
    pub value: Value,
    /// Occurrences.
    pub count: u64,
}

/// Summary of one column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnStatsReport {
    /// Total rows.
    pub rows: u64,
    /// `NULL` rows.
    pub nulls: u64,
    /// Distinct non-null values.
    pub distinct: u64,
    /// Most common values, descending.
    pub top: Vec<ValueCount>,
}

/// Bytes used by a group of keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefixUsage {
    /// Prefix, e.g. `cart:`.
    pub prefix: String,
    /// Keys.
    pub keys: u64,
    /// Bytes.
    pub bytes: u64,
}

/// Keys of one type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindUsage {
    /// Kind.
    pub kind: KeyKind,
    /// Keys.
    pub keys: u64,
}

/// Memory overview of a key-value store.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemoryReport {
    /// Total keys.
    pub total_keys: u64,
    /// Total bytes used.
    pub used_bytes: u64,
    /// Largest prefixes, descending by bytes. May be sampled.
    pub by_prefix: Vec<PrefixUsage>,
    /// Keys by type. May be sampled.
    pub by_kind: Vec<KindUsage>,
    /// True when the breakdowns come from a sample.
    pub sampled: bool,
}
