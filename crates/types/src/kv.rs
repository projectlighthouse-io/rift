//! Key-value data.

use serde::{Deserialize, Serialize};

use crate::KeyBytes;

/// Which keys a scan returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum KeyMatch {
    /// Every key.
    All,
    /// Keys starting with these bytes.
    Prefix(KeyBytes),
    /// Glob pattern such as `cart:*` (Redis).
    Pattern(String),
    /// Keys in `[begin, end)` (FoundationDB).
    Range {
        /// Inclusive start.
        begin: KeyBytes,
        /// Exclusive end.
        end: KeyBytes,
    },
}

/// A request for one page of keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyScan {
    /// Filter.
    pub matching: KeyMatch,
    /// Max keys per page (a hint for Redis `SCAN COUNT`).
    pub limit: u32,
    /// Iterate backwards where supported.
    pub reverse: bool,
    /// Continuation from the previous page.
    pub cursor: Option<ScanCursor>,
    /// Include system keys (FoundationDB `\xff`).
    pub include_system: bool,
}

/// Opaque continuation token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCursor(pub Vec<u8>);

/// Type of a stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum KeyKind {
    /// Plain bytes / string.
    String,
    /// Hash of fields.
    Hash,
    /// List.
    List,
    /// Set.
    Set,
    /// Sorted set.
    ZSet,
    /// Stream.
    Stream,
}

impl KeyKind {
    /// Upper-case badge label, e.g. `HASH`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            KeyKind::String => "STRING",
            KeyKind::Hash => "HASH",
            KeyKind::List => "LIST",
            KeyKind::Set => "SET",
            KeyKind::ZSet => "ZSET",
            KeyKind::Stream => "STREAM",
        }
    }
}

/// A key returned by a scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyEntry {
    /// The key.
    pub key: KeyBytes,
    /// Value type, when known.
    pub kind: Option<KeyKind>,
    /// Value size in bytes, when known.
    pub size: Option<u64>,
    /// Inline value for stores where reading it is free (FoundationDB).
    pub value: Option<Vec<u8>>,
}

/// One page of a scan.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct KeyPage {
    /// Keys.
    pub entries: Vec<KeyEntry>,
    /// `None` when the scan is finished.
    pub cursor: Option<ScanCursor>,
    /// Database version the page was read at (FoundationDB).
    pub read_version: Option<u64>,
}

/// A hash field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashField {
    /// Field name.
    pub field: Vec<u8>,
    /// Value.
    pub value: Vec<u8>,
}

/// A sorted-set member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredMember {
    /// Member.
    pub member: Vec<u8>,
    /// Score.
    pub score: f64,
}

/// A stream entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamEntry {
    /// Entry id, e.g. `1700000000000-0`.
    pub id: String,
    /// Fields.
    pub fields: Vec<HashField>,
}

/// A whole stored value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum KvValue {
    /// Bytes.
    String(Vec<u8>),
    /// Hash fields.
    Hash(Vec<HashField>),
    /// List items.
    List(Vec<Vec<u8>>),
    /// Set members.
    Set(Vec<Vec<u8>>),
    /// Sorted set.
    ZSet(Vec<ScoredMember>),
    /// Stream entries.
    Stream(Vec<StreamEntry>),
}

impl KvValue {
    /// The kind of this value.
    #[must_use]
    pub fn kind(&self) -> KeyKind {
        match self {
            KvValue::String(_) => KeyKind::String,
            KvValue::Hash(_) => KeyKind::Hash,
            KvValue::List(_) => KeyKind::List,
            KvValue::Set(_) => KeyKind::Set,
            KvValue::ZSet(_) => KeyKind::ZSet,
            KvValue::Stream(_) => KeyKind::Stream,
        }
    }
}

/// One write inside a transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TxOp {
    /// Run a SQL statement.
    Sql {
        /// Statement.
        sql: String,
    },
    /// Set a key.
    Set {
        /// Key.
        key: KeyBytes,
        /// Value.
        value: KvValue,
    },
    /// Delete a key.
    Delete {
        /// Key.
        key: KeyBytes,
    },
}

/// A directory of the FoundationDB directory layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyDirectory {
    /// Path from the root, e.g. `["app", "users"]`.
    pub path: Vec<String>,
    /// Key prefix of the directory's contents.
    pub prefix: KeyBytes,
}
