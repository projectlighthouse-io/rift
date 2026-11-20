//! App-wide services, available as a GPUI global.

use std::path::PathBuf;
use std::sync::Arc;

use conn::{ConnectionManager, KeychainSecretStore};
use gpui::{App, Global};
use store::SqliteStore;
use traits::ConnectDeps;

/// Services shared by every window.
pub(crate) struct AppState {
    pub(crate) manager: Arc<ConnectionManager>,
}

impl Global for AppState {}

/// Where the app database lives: `RIFT_DATA_DIR`, else the OS data directory.
fn data_dir() -> PathBuf {
    std::env::var_os("RIFT_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("rift")))
        .unwrap_or_else(|| PathBuf::from(".rift"))
}

/// Opens the store and builds the connection manager.
pub(crate) fn init(cx: &mut App) -> anyhow::Result<()> {
    let store = Arc::new(SqliteStore::open(&data_dir().join("rift.db"))?);
    let manager = ConnectionManager::new(
        registry::all(),
        store,
        Arc::new(KeychainSecretStore),
        ConnectDeps {
            spawner: crate::runtime::spawner(cx),
        },
    );
    cx.set_global(AppState {
        manager: Arc::new(manager),
    });
    Ok(())
}

/// The connection manager.
pub(crate) fn manager(cx: &App) -> Arc<ConnectionManager> {
    cx.global::<AppState>().manager.clone()
}
