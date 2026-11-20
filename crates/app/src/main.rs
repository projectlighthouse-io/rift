//! Rift: a fast, native, cross-platform database client.

mod actions;
mod assets;
mod connection_dialog;
mod connections;
mod dropdown;
mod export;
mod grid;
mod kv_view;
mod runtime;
mod session;
mod sql_editor;
mod sql_syntax;
mod state;
mod structure_view;
mod table_view;
mod theme;
mod ui;
mod workspace;

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Context as _;
use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;

use crate::runtime::{Io, TokioSpawner};
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
            actions::bind(cx);
            if let Err(e) = state::init(cx) {
                tracing::error!("opening app database: {e:#}");
                cx.quit();
                return;
            }
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
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        cx.new(|cx| Root::new(workspace, window, cx))
    });
    if let Err(e) = opened {
        tracing::error!("opening window: {e}");
    }
}
