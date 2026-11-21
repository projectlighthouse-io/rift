//! Connection manager, keychain-backed secrets, and (later) SSH tunnels and safe mode.

mod guard;
mod manager;
mod secrets;

pub use guard::ReadOnlyConnection;
pub use manager::ConnectionManager;
pub use secrets::KeychainSecretStore;
