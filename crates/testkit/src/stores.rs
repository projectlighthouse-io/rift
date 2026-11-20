//! In-memory app stores.

use std::sync::Mutex;

use traits::{ConnectionStore, DashboardStore, HistoryStore};
use types::{ConnectionId, Dashboard, DashboardId, Error, HistoryEntry, SavedConnection};

/// Connections in a vector.
#[derive(Debug, Default)]
pub struct MemoryConnectionStore {
    items: Mutex<Vec<SavedConnection>>,
}

impl ConnectionStore for MemoryConnectionStore {
    fn list(&self) -> Result<Vec<SavedConnection>, Error> {
        Ok(self.items.lock().expect("store poisoned").clone())
    }

    fn save(&self, connection: &SavedConnection) -> Result<(), Error> {
        let mut items = self.items.lock().expect("store poisoned");
        match items.iter_mut().find(|c| c.id == connection.id) {
            Some(existing) => *existing = connection.clone(),
            None => items.push(connection.clone()),
        }
        Ok(())
    }

    fn delete(&self, id: ConnectionId) -> Result<(), Error> {
        self.items
            .lock()
            .expect("store poisoned")
            .retain(|c| c.id != id);
        Ok(())
    }
}

/// History in a vector.
#[derive(Debug, Default)]
pub struct MemoryHistoryStore {
    items: Mutex<Vec<HistoryEntry>>,
}

impl HistoryStore for MemoryHistoryStore {
    fn append(&self, entry: &HistoryEntry) -> Result<(), Error> {
        self.items
            .lock()
            .expect("store poisoned")
            .push(entry.clone());
        Ok(())
    }

    fn recent(
        &self,
        connection: Option<ConnectionId>,
        limit: u32,
    ) -> Result<Vec<HistoryEntry>, Error> {
        let items = self.items.lock().expect("store poisoned");
        Ok(items
            .iter()
            .rev()
            .filter(|e| connection.is_none_or(|c| e.connection == c))
            .take(limit as usize)
            .cloned()
            .collect())
    }
}

/// Dashboards in a vector.
#[derive(Debug, Default)]
pub struct MemoryDashboardStore {
    items: Mutex<Vec<Dashboard>>,
}

impl DashboardStore for MemoryDashboardStore {
    fn list(&self, connection: ConnectionId) -> Result<Vec<Dashboard>, Error> {
        let items = self.items.lock().expect("store poisoned");
        Ok(items
            .iter()
            .filter(|d| d.connection == connection)
            .cloned()
            .collect())
    }

    fn save(&self, dashboard: &Dashboard) -> Result<(), Error> {
        let mut items = self.items.lock().expect("store poisoned");
        match items.iter_mut().find(|d| d.id == dashboard.id) {
            Some(existing) => *existing = dashboard.clone(),
            None => items.push(dashboard.clone()),
        }
        Ok(())
    }

    fn delete(&self, id: DashboardId) -> Result<(), Error> {
        self.items
            .lock()
            .expect("store poisoned")
            .retain(|d| d.id != id);
        Ok(())
    }
}
