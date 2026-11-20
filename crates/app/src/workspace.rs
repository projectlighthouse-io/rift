//! The window root: top bar with open connections, then the active session.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{Context, Entity, FontWeight, Window, canvas, div, px};

use crate::session::Session;
use crate::theme::{self, c};
use crate::ui;

/// Frames slower than this are logged in debug builds.
const FRAME_BUDGET_MS: u128 = 16;

/// `RIFT_FRAME_LOG=1` enables slow-frame warnings in release builds.
fn frame_log_enabled() -> bool {
    std::env::var_os("RIFT_FRAME_LOG").is_some()
}

/// Root view.
pub(crate) struct Workspace {
    sessions: Vec<Entity<Session>>,
    active: Option<usize>,
}

impl Workspace {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active: None,
        }
    }

    /// Adds a session and makes it active.
    pub(crate) fn add_session(&mut self, session: Entity<Session>, cx: &mut Context<Self>) {
        self.sessions.push(session);
        self.active = Some(self.sessions.len() - 1);
        cx.notify();
    }

    fn close_session(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.sessions.len() {
            let session = self.sessions.remove(ix);
            let conn = session.read(cx).conn.clone();
            crate::runtime::spawn_db(cx, async move { conn.close().await }, |_, _, _| {}).detach();
            self.active = match self.active {
                _ if self.sessions.is_empty() => None,
                Some(a) if a >= ix && a > 0 => Some(a - 1),
                other => other,
            };
            cx.notify();
        }
    }

    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let brand = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(36.))
            .pr(px(12.))
            .child(ui::icon("database", 18., theme::ACCENT_TEXT))
            .child(
                div()
                    .font_family(mono.clone())
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(c(theme::TEXT))
                    .child("rift"),
            );
        let tabs = div()
            .flex()
            .items_end()
            .gap(px(2.))
            .flex_1()
            .min_w_0()
            .children(self.sessions.iter().enumerate().map(|(ix, session)| {
                let s = session.read(cx);
                let active = self.active == Some(ix);
                let name = s.name.clone();
                div()
                    .id(("conn-tab", ix))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(37.))
                    .pl(px(12.))
                    .pr(px(8.))
                    .mb(px(-1.))
                    .rounded_tl(px(8.))
                    .rounded_tr(px(8.))
                    .border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .cursor_pointer()
                    .map(|d| {
                        if active {
                            d.bg(c(theme::BG)).border_color(c(theme::LINE))
                        } else {
                            d.border_color(gpui::transparent_black())
                        }
                    })
                    .text_color(c(if active { theme::TEXT } else { theme::MUTED }))
                    .child(ui::tag_square(s.tag, 8.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(theme::FAINT))
                            .child(s.engine()),
                    )
                    .when(s.read_only, |d| {
                        d.child(ui::badge(
                            "READ-ONLY",
                            theme::DELETED_TEXT,
                            theme::DELETED_BG,
                        ))
                    })
                    .child(
                        ui::icon_button(("close-conn", ix), "close", 22., 15.).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_session(ix, cx);
                            }),
                        ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = Some(ix);
                        cx.notify();
                    }))
            }))
            .child(
                div()
                    .mb(px(3.))
                    .child(ui::icon_button("open-conn", "plus", 30., 15.)),
            );
        let right = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(36.))
            .mb(px(4.))
            .child(
                div()
                    .id("open-anything")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(30.))
                    .w(px(220.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .bg(c(theme::RAISED))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .text_size(px(13.))
                    .text_color(c(theme::FAINT))
                    .cursor_pointer()
                    .child(ui::icon("search", 14., theme::FAINT))
                    .child(div().flex_1().child("Open anything…"))
                    .child(ui::kbd("⌘K", cx).text_color(c(theme::MUTED))),
            )
            .child(
                ui::secondary_button("sql", "SQL")
                    .child(ui::icon("terminal", 14., theme::TEXT))
                    .flex_row_reverse(),
            );
        div()
            .flex()
            .flex_none()
            .items_end()
            .gap(px(4.))
            .pt(px(6.))
            .px(px(12.))
            .bg(c(theme::BG))
            .border_b_1()
            .border_color(c(theme::LINE))
            .child(brand)
            .child(tabs)
            .child(right)
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();
        let body = match self.active.and_then(|ix| self.sessions.get(ix)) {
            Some(session) => session.clone().into_any_element(),
            None => div()
                .relative()
                .size_full()
                .child(ui::dot_grid())
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(c(theme::BG))
            .font_family(theme::SERIF)
            .text_size(px(13.))
            .text_color(c(theme::TEXT))
            .child(self.render_top_bar(cx))
            .child(div().flex_1().min_h_0().child(body))
            .child(
                canvas(
                    move |_, _, _| {
                        let ms = started.elapsed().as_millis();
                        if ms > FRAME_BUDGET_MS && (cfg!(debug_assertions) || frame_log_enabled()) {
                            tracing::warn!(ms, "slow frame");
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_0(),
            )
    }
}
