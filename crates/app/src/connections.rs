//! The start screen: saved connections grouped by Local / Staging / Production.

use gpui::prelude::*;
use gpui::{
    ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    SharedString, Subscription, Window, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::text::TextView;
use types::{FieldKind, SavedConnection, Tag};

use crate::actions::{EditConnection, NewConnection};
use crate::theme::{self, c};
use crate::{state, ui};

/// Groups shown first, in this order; others follow alphabetically.
pub(crate) const KNOWN_GROUPS: [&str; 3] = ["Local", "Staging", "Production"];

/// What the screen asks the workspace to do.
pub(crate) enum ConnectionsEvent {
    Connect(SavedConnection),
    New,
    Edit(SavedConnection),
}

/// Saved connections with search and keyboard navigation.
pub(crate) struct ConnectionsScreen {
    saved: Vec<SavedConnection>,
    search: Entity<InputState>,
    selected: usize,
    focus: FocusHandle,
    /// Error or progress line shown under the card.
    pub(crate) status: Option<(String, bool)>,
    _subscription: Subscription,
}

impl EventEmitter<ConnectionsEvent> for ConnectionsScreen {}

impl Focusable for ConnectionsScreen {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

/// `host:port`, a file path, or a cluster file: whatever identifies the server.
pub(crate) fn address(connection: &SavedConnection, cx: &gpui::App) -> String {
    let value = |k: &str| connection.values.get(&k.into()).filter(|v| !v.is_empty());
    if let Some(host) = value("host") {
        return match value("port") {
            Some(port) => format!("{host}:{port}"),
            None => host.clone(),
        };
    }
    let manager = state::manager(cx);
    let Some(driver) = manager.driver(connection.driver.as_str()) else {
        return String::new();
    };
    // A file path first, else the first field that has a value (e.g. a cluster string).
    let file = driver.fields.iter().filter(|f| f.kind == FieldKind::File);
    let rest = driver
        .fields
        .iter()
        .filter(|f| f.kind != FieldKind::Password);
    file.chain(rest)
        .find_map(|f| value(f.key.as_str()))
        .cloned()
        .unwrap_or_default()
}

/// Sort key putting known groups first.
pub(crate) fn group_rank(group: &str) -> (usize, String) {
    let rank = KNOWN_GROUPS
        .iter()
        .position(|g| *g == group)
        .unwrap_or(KNOWN_GROUPS.len());
    (rank, group.to_owned())
}

impl ConnectionsScreen {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search connections"));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.selected = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.connect_selected(cx),
                _ => {}
            },
        );
        search.update(cx, |s, cx| s.focus(window, cx));
        let mut screen = Self {
            saved: Vec::new(),
            search,
            selected: 0,
            focus: cx.focus_handle(),
            status: None,
            _subscription: subscription,
        };
        screen.reload(cx);
        screen
    }

    /// Re-reads saved connections from the store.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        match state::manager(cx).list() {
            Ok(mut saved) => {
                saved.sort_by_key(|s| group_rank(s.group.as_deref().unwrap_or("Other")));
                self.saved = saved;
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
        cx.notify();
    }

    /// Connections matching the search, in display order.
    fn visible(&self, cx: &gpui::App) -> Vec<&SavedConnection> {
        let query = self.search.read(cx).value().to_lowercase();
        self.saved
            .iter()
            .filter(|s| {
                query.is_empty()
                    || s.name.to_lowercase().contains(&query)
                    || address(s, cx).to_lowercase().contains(&query)
                    || s.driver.as_str().contains(&query)
            })
            .collect()
    }

    fn selected_connection(&self, cx: &gpui::App) -> Option<SavedConnection> {
        self.visible(cx).get(self.selected).map(|s| (*s).clone())
    }

    fn connect_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(conn) = self.selected_connection(cx) {
            cx.emit(ConnectionsEvent::Connect(conn));
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.visible(cx).len();
        match event.keystroke.key.as_str() {
            "up" if count > 0 => self.selected = self.selected.saturating_sub(1),
            "down" if count > 0 => self.selected = (self.selected + 1).min(count - 1),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_row(
        &self,
        ix: usize,
        conn: &SavedConnection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = ix == self.selected;
        let engine: SharedString = state::manager(cx)
            .driver(conn.driver.as_str())
            .map(|d| d.name.clone())
            .unwrap_or_else(|| conn.driver.to_string())
            .into();
        let picked = conn.clone();
        div()
            .id(("saved-conn", ix))
            .flex()
            .items_center()
            .gap(px(12.))
            .h(px(44.))
            .px(px(12.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(selected, |d| d.bg(c(theme::SELECTION)))
            .hover(|d| d.bg(c(theme::SELECTION)))
            .child(ui::tag_square(conn.tag.unwrap_or(Tag::Green), 10.))
            .child(
                div()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(SharedString::from(conn.name.clone())),
            )
            .child(
                div()
                    .font_family(theme::mono(cx))
                    .text_size(px(12.))
                    .text_color(c(theme::FAINT))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(address(conn, cx))),
            )
            .when(selected, |d| {
                let edit = conn.clone();
                d.child(div().ml_auto().child(
                    ui::ghost_button(("edit-conn", ix), "Edit").on_click(cx.listener(
                        move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(ConnectionsEvent::Edit(edit.clone()));
                        },
                    )),
                ))
            })
            .child(div().when(!selected, |d| d.ml_auto()).child(if selected {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(26.))
                    .px(px(10.))
                    .rounded(px(4.))
                    .bg(c(theme::PRIMARY))
                    .text_color(c(theme::BG))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Connect")
                    .child(ui::kbd("↵", cx).opacity(0.85))
            } else {
                div()
                    .text_size(px(12.))
                    .text_color(c(theme::FAINT))
                    .child(engine)
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = ix;
                cx.emit(ConnectionsEvent::Connect(picked.clone()));
            }))
    }
}

impl Render for ConnectionsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible: Vec<SavedConnection> = self.visible(cx).into_iter().cloned().collect();
        let mut card = div()
            .flex()
            .flex_col()
            .px(px(6.))
            .pb(px(8.))
            .bg(c(theme::BG))
            .border_1()
            .border_color(c(theme::LINE))
            .rounded(px(6.));
        let mut last_group: Option<String> = None;
        for (ix, conn) in visible.iter().enumerate() {
            let group = conn.group.clone().unwrap_or_else(|| "Other".into());
            if last_group.as_ref() != Some(&group) {
                card = card.child(
                    div()
                        .mt(px(16.))
                        .mb(px(4.))
                        .px(px(12.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c(theme::FAINT))
                        .child(SharedString::from(group.to_uppercase())),
                );
                last_group = Some(group);
            }
            card = card.child(div().mb(px(2.)).child(self.render_row(ix, conn, cx)));
        }
        if visible.is_empty() {
            card = card.child(
                div()
                    .py(px(28.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(c(theme::FAINT))
                    .child(if self.saved.is_empty() {
                        "No connections yet. Press ⌘N to add one."
                    } else {
                        "No connections match."
                    }),
            );
        }

        let header = div()
            .flex()
            .items_end()
            .gap(px(10.))
            .mb(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .text_color(c(theme::MUTED))
                            .child("/ connections"),
                    )
                    .child(
                        div()
                            .text_size(px(48.))
                            .font_weight(FontWeight::BOLD)
                            .line_height(px(53.))
                            .text_color(c(theme::TEXT))
                            .child("rift"),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .flex()
                            .text_size(px(17.))
                            .text_color(c(theme::MUTED))
                            .child("Pick a connection. Open\u{a0}")
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(c(theme::TEXT))
                                    .child("as many as you like"),
                            )
                            .child("."),
                    ),
            )
            .child(
                div().ml_auto().child(
                    div()
                        .id("new-connection")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(32.))
                        .px(px(12.))
                        .rounded(px(4.))
                        .bg(c(theme::BG))
                        .border_1()
                        .border_color(c(theme::LINE))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|d| d.bg(c(theme::RAISED)))
                        .child(ui::icon("plus", 14., theme::TEXT))
                        .child("New connection")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionsEvent::New))),
                ),
            );

        let search = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(40.))
            .pl(px(12.))
            .pr(px(8.))
            .rounded(px(4.))
            .bg(c(theme::BG))
            .border_1()
            .border_color(c(theme::LINE))
            .child(ui::icon("search", 14., theme::FAINT))
            .child(div().flex_1().child(ui::input(&self.search, 14.)));

        let hint = match &self.status {
            // ponytail: markdown view only because it is the selectable text element we have.
            Some((text, true)) => {
                let escaped: String = text
                    .chars()
                    .flat_map(|ch| {
                        let esc = ch.is_ascii_punctuation().then_some('\\');
                        esc.into_iter().chain([ch])
                    })
                    .collect();
                let copy = text.clone();
                div()
                    .flex()
                    .items_start()
                    .justify_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(c(theme::DELETED_TEXT))
                    .child(TextView::markdown("connect-error", escaped).selectable(true))
                    .child(ui::icon_button("copy-error", "copy", 20., 12.).on_click(
                        move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                        },
                    ))
            }
            Some((text, is_error)) => div()
                .text_size(px(12.))
                .text_center()
                .text_color(c(if *is_error {
                    theme::DELETED_TEXT
                } else {
                    theme::MUTED
                }))
                .child(SharedString::from(text.clone())),
            None => div()
                .text_size(px(12.))
                .text_center()
                .text_color(c(theme::FAINT))
                .child("Click a connection to open it · ↑↓ to move · ↵ to connect · ⌘E to edit"),
        };

        div()
            .id("connections")
            .key_context("Connections")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(72.))
            .px(px(24.))
            .overflow_y_scroll()
            .capture_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|_, _: &NewConnection, _, cx| cx.emit(ConnectionsEvent::New)))
            .on_action(cx.listener(|this, _: &EditConnection, _, cx| {
                if let Some(conn) = this.selected_connection(cx) {
                    cx.emit(ConnectionsEvent::Edit(conn));
                }
            }))
            .child(ui::dot_grid())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .w_full()
                    .max_w(px(640.))
                    .child(header)
                    .child(search)
                    .child(card)
                    .child(hint),
            )
    }
}
