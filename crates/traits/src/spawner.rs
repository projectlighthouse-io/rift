//! Running work off the UI thread.

use futures::channel::oneshot;
use futures::future::BoxFuture;
use types::Error;

/// Runs async and blocking work on the I/O runtime (Tokio in the app, inline in tests).
pub trait Spawner: Send + Sync {
    /// Spawns a future.
    fn spawn(&self, task: BoxFuture<'static, ()>);
    /// Runs blocking work (e.g. `rusqlite`, file I/O) on a blocking thread.
    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send + 'static>);
}

/// Runs `f` with [`Spawner::spawn_blocking`] and awaits its result.
///
/// # Errors
/// [`Error::Cancelled`] if the task was dropped before finishing (runtime shut down or panic).
pub async fn run_blocking<T, F>(spawner: &dyn Spawner, f: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    spawner.spawn_blocking(Box::new(move || {
        let _ = tx.send(f());
    }));
    rx.await.map_err(|_| Error::Cancelled)
}

/// Spawns `fut` with [`Spawner::spawn`] and awaits its output.
///
/// # Errors
/// [`Error::Cancelled`] if the task was dropped before finishing.
pub async fn run<T, F>(spawner: &dyn Spawner, fut: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    spawner.spawn(Box::pin(async move {
        let _ = tx.send(fut.await);
    }));
    rx.await.map_err(|_| Error::Cancelled)
}
