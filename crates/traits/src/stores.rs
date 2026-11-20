//! App data stores. Calls are synchronous (local SQLite); the app runs them off the UI thread.

use types::{ConnectionId, Dashboard, DashboardId, Error, HistoryEntry, SavedConnection};

/// Saved connections.
pub trait ConnectionStore: Send + Sync {
    /// All connections, ordered by group then name.
    fn list(&self) -> Result<Vec<SavedConnection>, Error>;
    /// Inserts or updates by id.
    fn save(&self, connection: &SavedConnection) -> Result<(), Error>;
    /// Deletes by id, with its history and dashboards.
    fn delete(&self, id: ConnectionId) -> Result<(), Error>;
}

/// Query history.
pub trait HistoryStore: Send + Sync {
    /// Appends an entry.
    fn append(&self, entry: &HistoryEntry) -> Result<(), Error>;
    /// Most recent first, optionally for one connection.
    fn recent(
        &self,
        connection: Option<ConnectionId>,
        limit: u32,
    ) -> Result<Vec<HistoryEntry>, Error>;
}

/// Dashboards.
pub trait DashboardStore: Send + Sync {
    /// Dashboards of a connection.
    fn list(&self, connection: ConnectionId) -> Result<Vec<Dashboard>, Error>;
    /// Inserts or updates by id.
    fn save(&self, dashboard: &Dashboard) -> Result<(), Error>;
    /// Deletes by id.
    fn delete(&self, id: DashboardId) -> Result<(), Error>;
}
