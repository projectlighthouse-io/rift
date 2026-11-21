//! The About window and the app menu.

use gpui::{
    App, Bounds, Context, FontWeight, SharedString, Window, WindowBounds, WindowOptions, actions,
    div, img, prelude::*, px, size,
};

use crate::theme::{self, c};

actions!(rift, [
    /// Show the About window.
    About,
    /// Quit Rift.
    Quit
]);

/// The app icon, rendered from `assets/icon/rift.svg` by `scripts/make-icon.sh`.
const ICON: &[u8] = include_bytes!("../assets/icon/rift-256.png");

/// Installs the Rift menu and its actions.
pub(crate) fn init(cx: &mut App) {
    cx.on_action(|_: &About, cx| open(cx));
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
    cx.set_menus([gpui::Menu {
        name: "Rift".into(),
        items: vec![
            gpui::MenuItem::action("About Rift", About),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Quit Rift", Quit),
        ],
        disabled: false,
    }]);
}

struct AboutView;

impl Render for AboutView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let image = std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, ICON.to_vec()));
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .bg(c(theme::BG))
            .font_family(theme::SERIF)
            .text_color(c(theme::TEXT))
            .child(img(image).size(px(96.)))
            .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child("Rift"))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(c(theme::MUTED))
                    .child(SharedString::from(format!("Version {}", env!("CARGO_PKG_VERSION")))),
            )
            .child(div().text_size(px(13.)).child("Rift — a Project Lighthouse project"))
    }
}

fn open(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(360.), px(280.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        is_resizable: false,
        ..WindowOptions::default()
    };
    if let Err(e) = cx.open_window(options, |_, cx| cx.new(|_| AboutView)) {
        tracing::error!("opening About: {e}");
    }
}

#[cfg(test)]
mod tests {
    /// Renders the app icon PNGs from the SVG. Run by `scripts/make-icon.sh`.
    #[test]
    #[ignore = "writes icon files; run scripts/make-icon.sh"]
    fn render_icon() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon");
        let svg = std::fs::read_to_string(dir.join("rift.svg")).expect("svg");
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).expect("parse");
        for size in [16u32, 32, 64, 128, 256, 512, 1024] {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("pixmap");
            let scale = size as f32 / 1024.0;
            resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
            pixmap.save_png(dir.join(format!("rift-{size}.png"))).expect("png");
        }
    }
}
