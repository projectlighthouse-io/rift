//! Connection configuration.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{DriverId, FieldKey};

/// A secret string whose `Debug` output is redacted. Never persisted by `store`.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Reveals the secret.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// How a connection guards against writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SafeMode {
    /// No protection.
    #[default]
    Off,
    /// Ask before every write.
    ConfirmWrites,
    /// Refuse all writes; also enforced in the database where possible.
    ReadOnly,
}

/// TLS preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TlsMode {
    /// Plain TCP.
    #[default]
    Disable,
    /// TLS if the server offers it.
    Prefer,
    /// TLS required, certificate verified.
    Require,
}

/// SSH tunnel settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConfig {
    /// Bastion host.
    pub host: String,
    /// Bastion port.
    pub port: u16,
    /// SSH user.
    pub user: String,
    /// Private key path; `None` uses the SSH agent or the password secret.
    pub key_path: Option<String>,
}

/// Everything a driver needs to connect. Built by `conn` from a saved connection plus the
/// keychain.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnConfig {
    /// Driver.
    pub driver: DriverId,
    /// Values for the driver's [`crate::FieldSpec`]s.
    pub values: BTreeMap<FieldKey, String>,
    /// Password from the keychain.
    pub password: Option<Secret>,
    /// TLS preference.
    pub tls: TlsMode,
    /// Safe mode; drivers enforce `ReadOnly` in the session when they can.
    pub safe_mode: SafeMode,
    /// Reach the server through this SSH host. Drivers never see it: `conn` opens the tunnel
    /// and points `host`/`port` at it.
    pub ssh: Option<SshConfig>,
}

impl ConnConfig {
    /// A config with no values set.
    #[must_use]
    pub fn new(driver: impl Into<DriverId>) -> Self {
        Self {
            driver: driver.into(),
            values: BTreeMap::new(),
            password: None,
            tls: TlsMode::Disable,
            safe_mode: SafeMode::Off,
            ssh: None,
        }
    }

    /// Sets a field value.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.values.insert(FieldKey::new(key), value.into());
        self
    }

    /// A field value, if set and non-empty.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(&FieldKey::new(key))
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}
