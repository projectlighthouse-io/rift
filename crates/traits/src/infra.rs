//! Secrets and SSH tunnels.

use futures::future::BoxFuture;
use types::{ConnectionId, Error, Secret, SshConfig};

/// Stores connection passwords outside the app database (the system keychain in the app).
pub trait SecretStore: Send + Sync {
    /// The password for a connection, if saved.
    fn get(&self, id: ConnectionId) -> Result<Option<Secret>, Error>;
    /// Saves or replaces the password.
    fn set(&self, id: ConnectionId, secret: &Secret) -> Result<(), Error>;
    /// Removes the password. Missing entries are not an error.
    fn delete(&self, id: ConnectionId) -> Result<(), Error>;
}

/// A running tunnel; closes when dropped.
pub trait TunnelHandle: Send + Sync {
    /// Local port forwarding to the target.
    fn local_port(&self) -> u16;
}

/// Opens SSH port forwards.
pub trait Tunnel: Send + Sync {
    /// Forwards a local port to `target_host:target_port` through the bastion.
    fn open<'a>(
        &'a self,
        ssh: &'a SshConfig,
        secret: Option<Secret>,
        target_host: &'a str,
        target_port: u16,
    ) -> BoxFuture<'a, Result<Box<dyn TunnelHandle>, Error>>;
}
