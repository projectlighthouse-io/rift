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

/// Dev only (`RIFT_DEV_INSECURE_SECRETS=1`): passwords in a plaintext JSON file next to the app
/// database, so automated UI checks never trigger keychain prompts on unsigned rebuilds.
struct FileSecretStore {
    path: PathBuf,
    lock: std::sync::Mutex<()>,
}

impl FileSecretStore {
    fn read(&self) -> std::collections::BTreeMap<String, String> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write(&self, map: &std::collections::BTreeMap<String, String>) -> Result<(), types::Error> {
        let json = serde_json::to_string(map).map_err(|e| types::Error::io(e.to_string()))?;
        std::fs::write(&self.path, json).map_err(|e| types::Error::io(e.to_string()))
    }
}

impl traits::SecretStore for FileSecretStore {
    fn get(&self, id: types::ConnectionId) -> Result<Option<types::Secret>, types::Error> {
        let _guard = self.lock.lock().expect("secret file lock");
        Ok(self.read().get(&id.to_string()).map(types::Secret::new))
    }

    fn set(&self, id: types::ConnectionId, secret: &types::Secret) -> Result<(), types::Error> {
        let _guard = self.lock.lock().expect("secret file lock");
        let mut map = self.read();
        map.insert(id.to_string(), secret.expose().to_owned());
        self.write(&map)
    }

    fn delete(&self, id: types::ConnectionId) -> Result<(), types::Error> {
        let _guard = self.lock.lock().expect("secret file lock");
        let mut map = self.read();
        map.remove(&id.to_string());
        self.write(&map)
    }
}

/// Opens the store and builds the connection manager.
pub(crate) fn init(cx: &mut App) -> anyhow::Result<()> {
    let store = Arc::new(SqliteStore::open(&data_dir().join("rift.db"))?);
    let manager = ConnectionManager::new(
        registry::all(),
        store,
        if std::env::var_os("RIFT_DEV_INSECURE_SECRETS").is_some() {
            tracing::warn!("RIFT_DEV_INSECURE_SECRETS: passwords are stored in plaintext");
            Arc::new(FileSecretStore {
                path: data_dir().join("dev-secrets.json"),
                lock: std::sync::Mutex::new(()),
            }) as Arc<dyn traits::SecretStore>
        } else {
            Arc::new(KeychainSecretStore)
        },
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
