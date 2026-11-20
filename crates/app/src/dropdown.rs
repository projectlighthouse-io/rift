//! A native-looking `<select>`: a bordered trigger with a chevron and a popup list.

use gpui::prelude::*;
use gpui::{Context, EventEmitter, FontWeight, SharedString, Window, anchored, deferred, div, px};

use crate::theme::{self, c};
use crate::ui;

/// Emitted when the user picks an option.
pub(crate) struct Changed;

/// Dropdown state.
pub(crate) struct Dropdown {
    options: Vec<SharedString>,
    selected: usize,
    open: bool,
    height: f32,
    mono: bool,
}

impl EventEmitter<Changed> for Dropdown {}

impl Dropdown {
    pub(crate) fn new(options: Vec<SharedString>, selected: usize) -> Self {
        Self {
            options,
            selected,
            open: false,
            height: 34.,
            mono: false,
        }
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn value(&self) -> SharedString {
        self.options.get(self.selected).cloned().unwrap_or_default()
    }

    pub(crate) fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.options.len() {
            self.selected = ix;
            self.open = false;
            cx.emit(Changed);
            cx.notify();
        }
    }
}

impl Render for Dropdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let font = |d: gpui::Div| {
            if self.mono {
                d.font_family(mono.clone()).text_size(px(12.5))
            } else {
                d
            }
        };
        let trigger = div()
            .id("dropdown-trigger")
            .flex()
            .items_center()
            .w_full()
            .h(px(self.height))
            .pl(px(8.))
            .pr(px(6.))
            .gap(px(6.))
            .rounded(px(4.))
            .bg(c(theme::BG))
            .border_1()
            .border_color(c(theme::LINE))
            .text_size(px(13.))
            .text_color(c(theme::TEXT))
            .cursor_pointer()
            .child(font(div().flex_1().overflow_hidden().whitespace_nowrap()).child(self.value()))
            .child(ui::icon("chevron-down", 14., theme::TEXT))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open = !this.open;
                cx.notify();
            }));
        let menu = self.open.then(|| {
            deferred(
                anchored().snap_to_window().child(
                    div()
                        .id("dropdown-menu")
                        .occlude()
                        .mt(px(2.))
                        .min_w(px(160.))
                        .max_h(px(280.))
                        .overflow_y_scroll()
                        .p(px(4.))
                        .rounded(px(6.))
                        .bg(c(theme::BG))
                        .border_1()
                        .border_color(c(theme::LINE))
                        .shadow_md()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.open = false;
                            cx.notify();
                        }))
                        .children(self.options.iter().enumerate().map(|(ix, option)| {
                            let selected = ix == self.selected;
                            font(div())
                                .id(("option", ix))
                                .flex()
                                .items_center()
                                .h(px(28.))
                                .px(px(8.))
                                .rounded(px(4.))
                                .text_size(px(13.))
                                .cursor_pointer()
                                .when(selected, |d| {
                                    d.bg(c(theme::SELECTION)).font_weight(FontWeight::MEDIUM)
                                })
                                .hover(|d| d.bg(c(theme::SELECTION)))
                                .child(option.clone())
                                .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                        })),
                ),
            )
            .with_priority(1)
        });
        div().relative().w_full().child(trigger).children(menu)
    }
}
