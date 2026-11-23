//! Bundled fonts and icons.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

/// Bundled font files, registered at startup.
pub(crate) const FONTS: [&[u8]; 7] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Italic.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"),
];

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        fn icon(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("rift/", $name, ".svg") => {
                    Some(include_bytes!(concat!("../assets/icons/", $name, ".svg")))
                })*
                _ => None,
            }
        }
    };
}

icons!(
    "bell",
    "chart",
    "check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "clock",
    "close",
    "code",
    "columns",
    "copy",
    "database",
    "download",
    "eye",
    "filter",
    "folder",
    "info",
    "key",
    "link",
    "more",
    "pin",
    "play",
    "plus",
    "refresh",
    "search",
    "table",
    "terminal",
    "trash",
    "upload",
);

/// Rift icons under `rift/`, falling back to gpui-component's bundled icons.
pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        match icon(path) {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        gpui_kit_assets::Assets.list(path)
    }
}
