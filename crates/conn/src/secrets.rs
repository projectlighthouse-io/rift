//! Passwords in the system keychain.

use traits::SecretStore;
use types::{ConnectionId, Error, Secret};

/// Keychain service name for every Rift secret.
const SERVICE: &str = "io.projectlighthouse.rift";

/// The OS keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux).
#[derive(Debug, Default)]
pub struct KeychainSecretStore;

fn entry(id: ConnectionId) -> Result<keyring::Entry, Error> {
    keyring::Entry::new(SERVICE, &id.to_string()).map_err(|e| Error::io(e.to_string()))
}

impl SecretStore for KeychainSecretStore {
    fn get(&self, id: ConnectionId) -> Result<Option<Secret>, Error> {
        match entry(id)?.get_password() {
            Ok(password) => Ok(Some(Secret::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::io(e.to_string())),
        }
    }

    fn set(&self, id: ConnectionId, secret: &Secret) -> Result<(), Error> {
        entry(id)?
            .set_password(secret.expose())
            .map_err(|e| Error::io(e.to_string()))
    }

    fn delete(&self, id: ConnectionId) -> Result<(), Error> {
        match entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::io(e.to_string())),
        }
    }
}
