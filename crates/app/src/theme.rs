//! Design tokens from `design/` (§4). Views take every colour and size from here.
//! The set is complete on purpose; some tokens are first used by later screens.
#![allow(dead_code)]

use gpui::{App, Hsla, Rgba, SharedString, rgb};

/// A hex colour token.
pub(crate) fn c(hex: u32) -> Rgba {
    rgb(hex)
}

/// Panels, cards, inputs.
pub(crate) const BG: u32 = 0xffffff;
/// Window background ("ground"): the top bar and sidebar sit on it.
pub(crate) const PANEL: u32 = 0xfafafa;
/// Badges, kbd, active pill tab.
pub(crate) const RAISED: u32 = 0xf4f4f5;
/// Lines and borders.
pub(crate) const LINE: u32 = 0xe4e4e7;
/// Text.
pub(crate) const TEXT: u32 = 0x18181b;
/// Muted text.
pub(crate) const MUTED: u32 = 0x52525b;
/// Faint text.
pub(crate) const FAINT: u32 = 0x71717a;
/// Links and accent text.
pub(crate) const ACCENT_TEXT: u32 = 0xc2453d;
/// Active tab underline, focus ring, chart marks.
pub(crate) const ACCENT: u32 = 0xe57368;
/// Primary button background.
pub(crate) const PRIMARY: u32 = 0x18181b;
/// Every other grid row.
pub(crate) const STRIPE: u32 = 0xfafaf9;
/// Selected rows and list items.
pub(crate) const SELECTION: u32 = 0xf4f4f5;
/// Edited cell background / text / outline.
pub(crate) const EDITED_BG: u32 = 0xfff8c5;
pub(crate) const EDITED_TEXT: u32 = 0x9a6700;
pub(crate) const EDITED_RING: u32 = 0xe2b552;
/// New row background / text.
pub(crate) const NEW_BG: u32 = 0xdafbe1;
pub(crate) const NEW_TEXT: u32 = 0x1a7f37;
/// Deleted row background / text.
pub(crate) const DELETED_BG: u32 = 0xffebe9;
pub(crate) const DELETED_TEXT: u32 = 0xd1242f;
/// Status dot for a live connection.
pub(crate) const CONNECTED: u32 = 0x22a35a;
/// Dot grid dots.
pub(crate) const DOT: u32 = 0xe4e4e7;
/// Dot grid spacing in pixels.
pub(crate) const DOT_SPACING: f32 = 22.0;

/// Font for all interface text and grid data (bundled).
pub(crate) const SANS: &str = "Inter";

/// Font only where code is read or written: SQL editor, SQL/DDL previews, JSON values (bundled).
const MONO: &str = "JetBrains Mono";

pub(crate) fn init(cx: &mut App) {
    // The SQL editor (gpui-component) reads its font from the component theme.
    let component = gpui_component::Theme::global_mut(cx);
    component.mono_font_family = MONO.into();
    component.mono_font_size = gpui::px(13.);
}

/// The code font family.
pub(crate) fn mono(_: &App) -> SharedString {
    MONO.into()
}

/// Converts a token to `Hsla`.
pub(crate) fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}
