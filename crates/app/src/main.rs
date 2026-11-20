//! Rift: a fast, native, cross-platform database client.

use gpui::prelude::*;
use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, div, px, size};

struct Hello;

impl Render for Hello {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui::white()).child("rift")
    }
}

fn main() {
    Application::with_platform(gpui_platform::current_platform(false)).run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Hello),
        )
        .expect("open window");
    });
}
