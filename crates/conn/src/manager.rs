//! Builds configs from saved connections and opens them.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use traits::{
    ConnectDeps, Connection, ConnectionStore, DriverFactory, SecretStore, Tunnel, TunnelHandle,
};
use types::{ConnConfig, ConnectionId, DriverInfo, Error, SafeMode, SavedConnection, Secret};

use crate::guard::ReadOnlyConnection;
use crate::ssh::RusshTunnel;

/// A tunnel kept open while its connection lives.
type OpenTunnel = (Weak<dyn Connection>, Box<dyn TunnelHandle>);

/// Owns the driver list, saved connections and secrets.
pub struct ConnectionManager {
    drivers: Vec<Box<dyn DriverFactory>>,
    store: Arc<dyn ConnectionStore>,
    secrets: Arc<dyn SecretStore>,
    deps: ConnectDeps,
    tunnel: Box<dyn Tunnel>,
    /// Open tunnels, kept while their connection lives.
    // ponytail: closed connections' tunnels are dropped on the next connect, not instantly.
    tunnels: Mutex<Vec<OpenTunnel>>,
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
            tunnel: Box::new(RusshTunnel::default()),
            tunnels: Mutex::new(Vec::new()),
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
            ssh: connection.ssh.clone(),
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
        let mut config = config.clone();
        let tunnel = match config.ssh.clone() {
            Some(ssh) => {
                let host = config.get("host").unwrap_or("127.0.0.1").to_owned();
                let port = match config.get("port") {
                    Some(p) => p
                        .parse()
                        .map_err(|_| Error::invalid(format!("bad port: {p}")))?,
                    None => driver
                        .info()
                        .default_port
                        .ok_or_else(|| Error::invalid("SSH needs a host and port"))?,
                };
                let handle = self.tunnel.open(&ssh, None, &host, port).await?;
                config = config
                    .with("host", "127.0.0.1")
                    .with("port", handle.local_port().to_string());
                Some(handle)
            }
            None => None,
        };
        let config = &config;
        let conn = driver.connect(config, &self.deps).await?;
        if let Some(handle) = tunnel {
            let mut tunnels = self
                .tunnels
                .lock()
                .map_err(|_| Error::invalid("tunnel list poisoned"))?;
            tunnels.retain(|(conn, _)| conn.strong_count() > 0);
            tunnels.push((Arc::downgrade(&conn), handle));
        }
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
