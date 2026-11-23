//! Design tokens from `design/` (§4). Views take every colour and size from here.
//! The set is complete on purpose; some tokens are first used by later screens.
#![allow(dead_code)]

use gpui::{App, Hsla, Rgba, SharedString, rgb};

/// A hex colour token.
pub(crate) fn c(hex: u32) -> Rgba {
    rgb(hex)
}

/// Page background.
pub(crate) const BG: u32 = 0xffffff;
/// Side panels, toolbars, status bars.
pub(crate) const PANEL: u32 = 0xfafaf9;
/// Secondary buttons, badges.
pub(crate) const RAISED: u32 = 0xf5f5f4;
/// Lines and borders.
pub(crate) const LINE: u32 = 0xe7e5e4;
/// Text.
pub(crate) const TEXT: u32 = 0x1c1917;
/// Muted text.
pub(crate) const MUTED: u32 = 0x57534e;
/// Faint text.
pub(crate) const FAINT: u32 = 0x6f6a65;
/// Links and accent text.
pub(crate) const ACCENT_TEXT: u32 = 0xc2453d;
/// Active tab underline, focus ring, chart marks.
pub(crate) const ACCENT: u32 = 0xe57368;
/// Primary button background.
pub(crate) const PRIMARY: u32 = 0x1c1917;
/// Selected rows and list items.
pub(crate) const SELECTION: u32 = 0xf2f0ed;
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
/// Dot grid dots.
pub(crate) const DOT: u32 = 0xdedbd7;
/// Dot grid spacing in pixels.
pub(crate) const DOT_SPACING: f32 = 22.0;

/// Soft connection tag colours, in [`types::Tag::ALL`] order.
pub(crate) fn tag_color(tag: types::Tag) -> u32 {
    match tag {
        types::Tag::Green => 0x7bb08a,
        types::Tag::Teal => 0x72aeb3,
        types::Tag::Blue => 0x88a6dc,
        types::Tag::Purple => 0xa597d8,
        types::Tag::Orange => 0xd6a06a,
        _ => 0xd98a85,
    }
}

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
