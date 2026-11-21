//! Builds configs from saved connections and opens them.

use std::sync::Arc;
use std::time::Duration;

use traits::{ConnectDeps, Connection, ConnectionStore, DriverFactory, SecretStore};
use types::{ConnConfig, ConnectionId, DriverInfo, Error, SafeMode, SavedConnection, Secret};

use crate::guard::ReadOnlyConnection;

/// Owns the driver list, saved connections and secrets.
pub struct ConnectionManager {
    drivers: Vec<Box<dyn DriverFactory>>,
    store: Arc<dyn ConnectionStore>,
    secrets: Arc<dyn SecretStore>,
    deps: ConnectDeps,
}

impl ConnectionManager {
    /// A manager over the given drivers and stores.
    #[must_use]
    pub fn new(
        drivers: Vec<Box<dyn DriverFactory>>,
        store: Arc<dyn ConnectionStore>,
        secrets: Arc<dyn SecretStore>,
        deps: ConnectDeps,
    ) -> Self {
        Self {
            drivers,
            store,
            secrets,
            deps,
        }
    }

    /// Every registered driver, in picker order.
    pub fn drivers(&self) -> impl Iterator<Item = &DriverInfo> {
        self.drivers.iter().map(|d| d.info())
    }

    /// The driver with this id.
    #[must_use]
    pub fn driver(&self, id: &str) -> Option<&DriverInfo> {
        self.drivers().find(|d| d.id.as_str() == id)
    }

    /// Saved connections.
    ///
    /// # Errors
    /// Store failures.
    pub fn list(&self) -> Result<Vec<SavedConnection>, Error> {
        self.store.list()
    }

    /// Saves a connection, and its password when given (`Some("")` removes it).
    ///
    /// # Errors
    /// Store or keychain failures.
    pub fn save(
        &self,
        connection: &SavedConnection,
        password: Option<&Secret>,
    ) -> Result<(), Error> {
        self.store.save(connection)?;
        match password {
            Some(p) if p.expose().is_empty() => self.secrets.delete(connection.id),
            Some(p) => self.secrets.set(connection.id, p),
            None => Ok(()),
        }
    }

    /// Deletes a connection and its password.
    ///
    /// # Errors
    /// Store or keychain failures.
    pub fn delete(&self, id: ConnectionId) -> Result<(), Error> {
        self.store.delete(id)?;
        self.secrets.delete(id)
    }

    /// The saved password, if any.
    ///
    /// # Errors
    /// Keychain failures.
    pub fn password(&self, id: ConnectionId) -> Result<Option<Secret>, Error> {
        self.secrets.get(id)
    }

    /// The config a driver needs for a saved connection.
    #[must_use]
    pub fn config(connection: &SavedConnection, password: Option<Secret>) -> ConnConfig {
        ConnConfig {
            driver: connection.driver.clone(),
            values: connection.values.clone(),
            password,
            tls: connection.tls,
            safe_mode: connection.safe_mode,
        }
    }

    /// Opens a connection. Run on the I/O runtime.
    ///
    /// # Errors
    /// Unknown driver, or the driver's connect error.
    pub async fn connect(&self, config: &ConnConfig) -> Result<Arc<dyn Connection>, Error> {
        let driver = self
            .drivers
            .iter()
            .find(|d| d.info().id == config.driver)
            .ok_or_else(|| Error::invalid(format!("unknown driver {}", config.driver)))?;
        let conn = driver.connect(config, &self.deps).await?;
        Ok(if config.safe_mode == SafeMode::ReadOnly {
            Arc::new(ReadOnlyConnection::new(conn, config.driver.as_str()))
        } else {
            conn
        })
    }

    /// Connects, pings and closes: the New connection dialog's Test button.
    ///
    /// # Errors
    /// The connect or ping error.
    pub async fn test(&self, config: &ConnConfig) -> Result<Duration, Error> {
        let conn = self.connect(config).await?;
        let result = conn.ping().await;
        let _ = conn.close().await;
        result
    }
}
