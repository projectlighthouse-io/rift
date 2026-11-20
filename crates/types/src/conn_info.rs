//! Live connection metadata.

use serde::{Deserialize, Serialize};

use crate::DriverId;

/// Facts about an open connection, shown in the status bar and sidebar footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Driver.
    pub driver: DriverId,
    /// Server version, e.g. `PostgreSQL 16.4`.
    pub server_version: String,
    /// Address shown to the user, e.g. `127.0.0.1:5432` or a file path.
    pub address: String,
    /// Current database, if the engine has one.
    pub database: Option<String>,
}
