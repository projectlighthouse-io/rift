//! Small result types: explain plans and backups.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A query plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainPlan {
    /// Plan as text, one node per line.
    pub text: String,
}

/// Result of a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupReport {
    /// Where it was written.
    pub path: PathBuf,
    /// Size in bytes.
    pub bytes: u64,
}
