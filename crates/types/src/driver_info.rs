//! Driver metadata used to build the New connection form.

use serde::{Deserialize, Serialize};

use crate::{DriverId, FieldKey};

/// Broad database family. The UI never branches on this; it only groups drivers in pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Family {
    /// Relational, SQL.
    Sql,
    /// Key-value.
    KeyValue,
}

/// Widget used for a connection field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum FieldKind {
    /// Free text.
    Text,
    /// Host name or IP.
    Host,
    /// TCP port.
    Port,
    /// Secret, stored in the system keychain.
    Password,
    /// Path to a file, picked with a file dialog.
    File,
    /// On/off.
    Bool,
    /// One of a fixed set of options.
    Select {
        /// Allowed values.
        options: Vec<String>,
    },
}

/// One field on the connection form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSpec {
    /// Key in [`crate::ConnConfig::values`].
    pub key: FieldKey,
    /// Label, e.g. `Host`.
    pub label: String,
    /// Widget.
    pub kind: FieldKind,
    /// Must be non-empty to connect.
    pub required: bool,
    /// Prefilled value.
    pub default: Option<String>,
    /// Hint shown when empty.
    pub placeholder: Option<String>,
    /// Shown under "More options".
    pub advanced: bool,
}

impl FieldSpec {
    /// A required, basic field.
    #[must_use]
    pub fn new(key: &str, label: &str, kind: FieldKind) -> Self {
        Self {
            key: FieldKey::new(key),
            label: label.to_owned(),
            kind,
            required: false,
            default: None,
            placeholder: None,
            advanced: false,
        }
    }

    /// Marks the field required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Sets the default.
    #[must_use]
    pub fn default_value(mut self, value: &str) -> Self {
        self.default = Some(value.to_owned());
        self
    }

    /// Sets the placeholder.
    #[must_use]
    pub fn placeholder(mut self, value: &str) -> Self {
        self.placeholder = Some(value.to_owned());
        self
    }

    /// Moves the field under "More options".
    #[must_use]
    pub fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }
}

/// Static description of a driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverInfo {
    /// Stable id, e.g. `postgres`.
    pub id: DriverId,
    /// Display name, e.g. `PostgreSQL`.
    pub name: String,
    /// Family.
    pub family: Family,
    /// Form fields, in display order.
    pub fields: Vec<FieldSpec>,
    /// URL schemes accepted on paste, e.g. `postgres`, `postgresql`.
    pub url_schemes: Vec<String>,
    /// Default TCP port.
    pub default_port: Option<u16>,
}
