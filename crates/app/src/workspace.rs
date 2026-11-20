//! The window root: top bar with open connections, then the active session or the start
//! screen (connections list / connection dialog).

use std::time::Instant;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, FontWeight, Subscription, Window, canvas, div, px};
use types::{SafeMode, SavedConnection, Tag};

use crate::connection_dialog::{ConnectionDialog, DialogEvent};
use crate::connections::{ConnectionsEvent, ConnectionsScreen};
use crate::session::Session;
use crate::theme::{self, c};
use crate::{state, ui};

/// Frames slower than this are logged in debug builds.
const FRAME_BUDGET_MS: u128 = 16;

/// `RIFT_FRAME_LOG=1` enables slow-frame warnings in release builds.
fn frame_log_enabled() -> bool {
    std::env::var_os("RIFT_FRAME_LOG").is_some()
}

enum Start {
    List(Entity<ConnectionsScreen>),
    Dialog(Entity<ConnectionDialog>),
}

/// Root view.
pub(crate) struct Workspace {
    sessions: Vec<Entity<Session>>,
    active: Option<usize>,
    start: Option<Start>,
    _start_subscription: Option<Subscription>,
}

impl Workspace {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut workspace = Self {
            sessions: Vec::new(),
            active: None,
            start: None,
            _start_subscription: None,
        };
        workspace.show_list(window, cx);
        workspace
    }

    /// Shows the connections list.
    fn show_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let screen = cx.new(|cx| ConnectionsScreen::new(window, cx));
        self._start_subscription = Some(cx.subscribe_in(
            &screen,
            window,
            |this, _, event: &ConnectionsEvent, window, cx| match event {
                ConnectionsEvent::Connect(saved) => this.connect(saved.clone(), window, cx),
                ConnectionsEvent::New => this.show_dialog(None, window, cx),
                ConnectionsEvent::Edit(saved) => this.show_dialog(Some(saved.clone()), window, cx),
            },
        ));
        self.start = Some(Start::List(screen));
        cx.notify();
    }

    fn show_dialog(
        &mut self,
        editing: Option<SavedConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let groups: Vec<String> = state::manager(cx)
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| s.group)
            .collect();
        let dialog = cx.new(|cx| ConnectionDialog::new(editing, groups, window, cx));
        self._start_subscription = Some(cx.subscribe_in(
            &dialog,
            window,
            |this, _, event: &DialogEvent, window, cx| match event {
                DialogEvent::Saved(connection) => {
                    let connection = (**connection).clone();
                    this.show_list(window, cx);
                    this.connect(connection, window, cx);
                }
                DialogEvent::Deleted | DialogEvent::Cancelled => this.show_list(window, cx),
            },
        ));
        self.start = Some(Start::Dialog(dialog));
        cx.notify();
    }

    fn set_status(&self, text: String, is_error: bool, cx: &mut Context<Self>) {
        if let Some(Start::List(screen)) = &self.start {
            screen.update(cx, |s, cx| {
                s.status = Some((text, is_error));
                cx.notify();
            });
        }
    }

    /// Connects on the I/O runtime, then opens a session tab.
    fn connect(&mut self, saved: SavedConnection, window: &mut Window, cx: &mut Context<Self>) {
        self.set_status(format!("Connecting to {}…", saved.name), false, cx);
        let manager = state::manager(cx);
        let (tx, rx) = futures::channel::oneshot::channel();
        let target = saved.clone();
        cx.global::<crate::runtime::Io>().0.spawn_task(async move {
            let password = manager.password(target.id);
            let result = match password {
                Ok(password) => {
                    manager
                        .connect(&conn::ConnectionManager::config(&target, password))
                        .await
                }
                Err(e) => Err(e),
            };
            let _ = tx.send(result);
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else { return };
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(conn) => {
                    let read_only = saved.safe_mode == SafeMode::ReadOnly;
                    let tag = saved.tag.unwrap_or(Tag::Green);
                    let session = cx.new(|cx| {
                        let mut s = Session::new(
                            saved.name.clone().into(),
                            tag,
                            read_only,
                            conn,
                            window,
                            cx,
                        );
                        // Dev check: `RIFT_DEV_QUERY` opens a result tab (100k-row scroll test).
                        if let Ok(sql) = std::env::var("RIFT_DEV_QUERY") {
                            s.open_query(sql, cx);
                        }
                        s
                    });
                    this.sessions.push(session);
                    this.active = Some(this.sessions.len() - 1);
                    this.start = None;
                    this._start_subscription = None;
                    cx.notify();
                }
                Err(e) => this.set_status(format!("{}: {e}", saved.name), true, cx),
            });
        })
        .detach();
    }

    fn close_session(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.sessions.len() {
            let session = self.sessions.remove(ix);
            let conn = session.read(cx).conn.clone();
            crate::runtime::spawn_db(cx, async move { conn.close().await }, |_, _, _| {}).detach();
            self.active = match self.active {
                _ if self.sessions.is_empty() => None,
                Some(a) if a >= ix && a > 0 => Some(a - 1),
                other => other,
            };
            if self.sessions.is_empty() {
                self.show_list(window, cx);
            }
            cx.notify();
        }
    }

    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let showing_start = self.start.is_some();
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
                let active = !showing_start && self.active == Some(ix);
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
                            .child(s.name.clone()),
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
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_session(ix, window, cx);
                            }),
                        ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = Some(ix);
                        this.start = None;
                        this._start_subscription = None;
                        cx.notify();
                    }))
            }))
            .child(
                div().mb(px(3.)).child(
                    ui::icon_button("open-conn", "plus", 30., 15.)
                        .on_click(cx.listener(|this, _, window, cx| this.show_list(window, cx))),
                ),
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
        let body: AnyElement = match (
            &self.start,
            self.active.and_then(|ix| self.sessions.get(ix)),
        ) {
            (Some(Start::List(screen)), _) => screen.clone().into_any_element(),
            (Some(Start::Dialog(dialog)), _) => dialog.clone().into_any_element(),
            (None, Some(session)) => session.clone().into_any_element(),
            (None, None) => div()
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
            .when(!self.sessions.is_empty(), |d| {
                d.child(self.render_top_bar(cx))
            })
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
