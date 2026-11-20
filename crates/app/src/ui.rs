//! Small styled building blocks shared by every screen.

use gpui::prelude::*;
use gpui::{
    App, Bounds, Div, ElementId, FontWeight, Pixels, SharedString, Stateful, Styled, Svg, canvas,
    div, point, px, size, svg,
};

use crate::theme::{self, c};

/// One of the design's 24×24 stroke icons (`assets/icons`).
pub(crate) fn icon(name: &str, size_px: f32, color: u32) -> Svg {
    svg()
        .path(SharedString::from(format!("rift/{name}.svg")))
        .size(px(size_px))
        .flex_none()
        .text_color(c(color))
}

/// A small rounded square in a connection's tag colour.
pub(crate) fn tag_square(tag: types::Tag, size_px: f32) -> Div {
    div()
        .size(px(size_px))
        .flex_none()
        .rounded(px(2.))
        .bg(c(theme::tag_color(tag)))
}

/// A pill such as `READ-ONLY` or `2 edited`.
pub(crate) fn badge(text: impl Into<SharedString>, fg: u32, bg: u32) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(fg))
        .bg(c(bg))
        .whitespace_nowrap()
        .child(text.into())
}

/// Base for every button: a clickable row with keyboard-visible hover.
fn button_base(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(6.))
        .cursor_pointer()
        .whitespace_nowrap()
}

/// Raised secondary button (`Apply`, `SQL`).
pub(crate) fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    button_base(id)
        .h(px(28.))
        .px(px(10.))
        .bg(c(theme::RAISED))
        .border_1()
        .border_color(c(theme::LINE))
        .text_color(c(theme::TEXT))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .hover(|s| s.bg(c(theme::SELECTION)))
        .child(label.into())
}

/// Square icon-only button.
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    name: &str,
    box_px: f32,
    icon_px: f32,
) -> Stateful<Div> {
    button_base(id)
        .size(px(box_px))
        .text_color(c(theme::MUTED))
        .hover(|s| s.bg(c(theme::RAISED)))
        .child(icon(name, icon_px, theme::MUTED))
}

/// Keyboard hint such as `⌘K`.
pub(crate) fn kbd(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .font_family(theme::mono(cx))
        .text_size(px(11.))
        .child(text.into())
}

/// Faint 2px dots every 22px, painted behind content. Place as the first, absolutely
/// positioned child of a relative container.
pub(crate) fn dot_grid() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds: Bounds<Pixels>, (), window, _| {
            let color = theme::hsla(theme::DOT);
            let step = theme::DOT_SPACING;
            let half = step / 2.0;
            let cols = (f32::from(bounds.size.width) / step).ceil() as usize;
            let rows = (f32::from(bounds.size.height) / step).ceil() as usize;
            for row in 0..rows {
                for col in 0..cols {
                    let origin = point(
                        bounds.origin.x + px(half + col as f32 * step - 1.0),
                        bounds.origin.y + px(half + row as f32 * step - 1.0),
                    );
                    window.paint_quad(
                        gpui::fill(Bounds::new(origin, size(px(2.), px(2.))), color)
                            .corner_radii(px(1.)),
                    );
                }
            }
        },
    )
    .absolute()
    .size_full()
}

/// `1284` → `1,284`.
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn thousands_groups_digits() {
        assert_eq!(super::thousands(1284), "1,284");
        assert_eq!(super::thousands(48211), "48,211");
        assert_eq!(super::thousands(7), "7");
        assert_eq!(super::thousands(1_000_000), "1,000,000");
    }
}
