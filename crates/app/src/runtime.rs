//! The Tokio runtime and the bridge between it and GPUI.
//!
//! GPUI's thread only renders. Database futures run on Tokio; their results come back over a
//! oneshot channel and are applied with `this.update(...)`.

use std::future::Future;
use std::sync::Arc;

use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::{App, Context, Global, Task};
use tokio::runtime::Handle;
use traits::Spawner;

/// Runs futures and blocking work on the `rift-io` Tokio runtime.
pub(crate) struct TokioSpawner {
    handle: Handle,
}

impl TokioSpawner {
    pub(crate) fn new(handle: Handle) -> Self {
        Self { handle }
    }

    /// Spawns a future on the I/O runtime.
    pub(crate) fn spawn_task(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.handle.spawn(task);
    }
}

impl Spawner for TokioSpawner {
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        self.handle.spawn(task);
    }

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send + 'static>) {
        self.handle.spawn_blocking(task);
    }
}

/// The app-wide spawner.
pub(crate) struct Io(pub(crate) Arc<TokioSpawner>);

impl Global for Io {}

/// The spawner as a trait object, for `ConnectDeps`.
pub(crate) fn spawner(cx: &App) -> Arc<dyn Spawner> {
    cx.global::<Io>().0.clone()
}

/// Runs `fut` on Tokio and applies its output to the view with `on_done` on the UI thread.
/// Dropping the returned task stops `on_done` from running (the future still completes).
pub(crate) fn spawn_db<V, T>(
    cx: &mut Context<V>,
    fut: impl Future<Output = T> + Send + 'static,
    on_done: impl FnOnce(&mut V, T, &mut Context<V>) + 'static,
) -> Task<()>
where
    V: 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    cx.global::<Io>().0.spawn(Box::pin(async move {
        let _ = tx.send(fut.await);
    }));
    cx.spawn(async move |this, cx| {
        if let Ok(value) = rx.await {
            let _ = this.update(cx, |view, cx| on_done(view, value, cx));
        }
    })
}
