//! Connection manager, keychain-backed secrets, and (later) SSH tunnels and safe mode.

mod manager;
mod secrets;

pub use manager::ConnectionManager;
pub use secrets::KeychainSecretStore;
