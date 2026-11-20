//! Records persisted by the app database.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    CardId, ChartSpec, ConnectionId, DashboardId, DriverId, FieldKey, SafeMode, SshConfig, TlsMode,
};
use std::collections::BTreeMap;

/// Soft colour tag on a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Tag {
    /// Green.
    Green,
    /// Teal.
    Teal,
    /// Blue.
    Blue,
    /// Purple.
    Purple,
    /// Orange.
    Orange,
    /// Red.
    Red,
}

impl Tag {
    /// All tags in picker order.
    pub const ALL: [Tag; 6] = [
        Tag::Green,
        Tag::Teal,
        Tag::Blue,
        Tag::Purple,
        Tag::Orange,
        Tag::Red,
    ];
}

/// A saved connection. The password lives in the keychain, never here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedConnection {
    /// Id.
    pub id: ConnectionId,
    /// Display name, e.g. `local-dev`.
    pub name: String,
    /// Driver.
    pub driver: DriverId,
    /// Group, e.g. `Local`, `Staging`, `Production`.
    pub group: Option<String>,
    /// Colour tag.
    pub tag: Option<Tag>,
    /// Field values (no secrets).
    pub values: BTreeMap<FieldKey, String>,
    /// TLS.
    pub tls: TlsMode,
    /// Safe mode.
    pub safe_mode: SafeMode,
    /// SSH tunnel.
    pub ssh: Option<SshConfig>,
}

/// One executed statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Connection it ran on.
    pub connection: ConnectionId,
    /// SQL text.
    pub sql: String,
    /// When it started.
    pub ran_at: DateTime<Utc>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Rows returned or affected.
    pub rows: Option<u64>,
    /// Error message if it failed.
    pub error: Option<String>,
}

/// Comparison in an alert rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AlertOp {
    /// Fire when below.
    Below,
    /// Fire when above.
    Above,
}

/// Raise a notification when a card's latest value crosses a threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertRule {
    /// Comparison.
    pub op: AlertOp,
    /// Threshold.
    pub threshold: f64,
    /// Badge text, e.g. `Alert if < 30/day`.
    pub label: String,
}

/// A chart pinned to a dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartCard {
    /// Id.
    pub id: CardId,
    /// Title.
    pub title: String,
    /// Subtitle.
    pub subtitle: String,
    /// Chart.
    pub chart: ChartSpec,
    /// Refresh interval in seconds; `None` for manual.
    pub refresh_secs: Option<u32>,
    /// Alert.
    pub alert: Option<AlertRule>,
}

/// A dashboard of chart cards for one connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    /// Id.
    pub id: DashboardId,
    /// Connection.
    pub connection: ConnectionId,
    /// Title.
    pub title: String,
    /// Cards in grid order.
    pub cards: Vec<ChartCard>,
}
