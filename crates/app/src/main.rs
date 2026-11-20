//! Rift: a fast, native, cross-platform database client.

mod assets;
mod grid;
mod result_view;
mod runtime;
mod session;
mod table_view;
mod theme;
mod ui;
mod workspace;

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Context as _;
use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;
use traits::ConnectDeps;
use types::{ConnConfig, Secret, TableRef, Tag};

use crate::runtime::{Io, TokioSpawner};
use crate::session::Session;
use crate::workspace::Workspace;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("rift-io")
        .enable_all()
        .build()
        .context("start tokio runtime")?;
    let spawner = Arc::new(TokioSpawner::new(runtime.handle().clone()));

    Application::with_platform(gpui_platform::current_platform(false))
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            cx.set_global(Io(spawner));
            if let Err(e) = cx
                .text_system()
                .add_fonts(assets::FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
            {
                tracing::error!("loading fonts: {e}");
            }
            gpui_component::init(cx);
            theme::init(cx);
            open_main_window(cx);
            cx.activate(true);
        });
    drop(runtime);
    Ok(())
}

fn open_main_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let workspace = cx.new(|_| Workspace::new());
        dev_connect(workspace.clone(), window, cx);
        cx.new(|cx| Root::new(workspace, window, cx))
    });
    if let Err(e) = opened {
        tracing::error!("opening window: {e}");
    }
}

/// M2: connect straight to the local Docker Postgres. The Connections screen replaces this.
fn dev_connect(workspace: gpui::Entity<Workspace>, window: &mut gpui::Window, cx: &mut App) {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_owned());
    let mut config = ConnConfig::new("postgres")
        .with("host", env("RIFT_PG_HOST", "127.0.0.1"))
        .with("port", env("RIFT_PG_PORT", "5432"))
        .with("user", "postgres")
        .with("database", "shop_dev");
    config.password = Some(Secret::new("postgres"));
    let deps = ConnectDeps {
        spawner: runtime::spawner(cx),
    };
    let factories = registry::all();
    let window_handle = window.window_handle();
    let (tx, rx) = futures::channel::oneshot::channel();
    cx.global::<Io>().0.spawn_task(async move {
        let Some(factory) = factories
            .iter()
            .find(|f| f.info().id.as_str() == "postgres")
        else {
            return;
        };
        let _ = tx.send(factory.connect(&config, &deps).await);
    });
    cx.spawn(async move |cx| {
        let Ok(result) = rx.await else { return };
        let conn = match result {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("connect: {e}");
                return;
            }
        };
        let _ = window_handle.update(cx, |_, window, cx| {
            let session = cx.new(|cx| {
                let mut s = Session::new("local-dev".into(), Tag::Green, false, conn, window, cx);
                s.open_table(TableRef::new(Some("public".into()), "orders"), cx);
                if let Ok(sql) = std::env::var("RIFT_DEV_QUERY") {
                    s.open_query(sql, cx);
                }
                s
            });
            workspace.update(cx, |w, cx| w.add_session(session, cx));
        });
    })
    .detach();
}
