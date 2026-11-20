//! String and byte newtypes for identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wraps a string.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Borrows the inner string.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

string_newtype!(
    /// A table or view name, unquoted.
    TableName
);
string_newtype!(
    /// A schema (Postgres) or database (MySQL) name, unquoted.
    SchemaName
);
string_newtype!(
    /// A column name, unquoted.
    ColumnName
);
string_newtype!(
    /// An index name, unquoted.
    IndexName
);
string_newtype!(
    /// Stable driver identifier, e.g. `postgres`.
    DriverId
);
string_newtype!(
    /// Key of a [`crate::FieldSpec`], e.g. `host`.
    FieldKey
);

/// A raw key in a key-value store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyBytes(Vec<u8>);

impl KeyBytes {
    /// Wraps raw bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Borrows the bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the key and returns its bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Lossy UTF-8 rendering for display.
    #[must_use]
    pub fn to_display(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

impl From<&str> for KeyBytes {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}
