//! Driver factories.

use std::sync::Arc;

use futures::future::BoxFuture;
use types::{ConnConfig, DriverInfo, Error};

use crate::{Connection, Spawner};

/// Infrastructure handed to a driver when it connects.
#[derive(Clone)]
pub struct ConnectDeps {
    /// Runs futures and blocking work on the I/O runtime.
    pub spawner: Arc<dyn Spawner>,
}

/// Creates connections for one database engine. Each driver crate exposes exactly one, via
/// `pub fn factory() -> Box<dyn DriverFactory>`.
pub trait DriverFactory: Send + Sync {
    /// Id, name, family and connection form fields.
    fn info(&self) -> &DriverInfo;
    /// Opens a connection. Must run on the I/O runtime.
    fn connect<'a>(
        &'a self,
        config: &'a ConnConfig,
        deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>>;
}
