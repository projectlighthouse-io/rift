//! In-memory infrastructure fakes.

use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;

use futures::future::BoxFuture;
use traits::{SecretStore, Spawner};
use types::{ConnectionId, Error, Secret};

/// Runs every task on its own OS thread. Good enough for tests; no runtime needed.
#[derive(Debug, Default)]
pub struct ThreadSpawner;

impl Spawner for ThreadSpawner {
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        thread::spawn(move || futures::executor::block_on(task));
    }

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send + 'static>) {
        thread::spawn(task);
    }
}

/// Secrets in a map.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    secrets: Mutex<HashMap<ConnectionId, Secret>>,
}

impl SecretStore for MemorySecretStore {
    fn get(&self, id: ConnectionId) -> Result<Option<Secret>, Error> {
        Ok(self
            .secrets
            .lock()
            .expect("secret map poisoned")
            .get(&id)
            .cloned())
    }

    fn set(&self, id: ConnectionId, secret: &Secret) -> Result<(), Error> {
        self.secrets
            .lock()
            .expect("secret map poisoned")
            .insert(id, secret.clone());
        Ok(())
    }

    fn delete(&self, id: ConnectionId) -> Result<(), Error> {
        self.secrets
            .lock()
            .expect("secret map poisoned")
            .remove(&id);
        Ok(())
    }
}
