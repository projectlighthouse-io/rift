//! ⌘K command palette: one search over every open connection's tables and views,
//! dashboards, query history, and connect actions for saved connections.

use std::ops::Range;

use chrono::{Local, Utc};
use gpui::prelude::*;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    KeyDownEvent, SharedString, StyledText, Subscription, Window, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use types::{Dashboard, ObjectKind, SavedConnection, TableRef};

use crate::theme::{self, c};
use crate::ui;

/// Results shown at once; the search itself covers everything.
const MAX_RESULTS: usize = 50;

/// What a result opens.
#[derive(Clone)]
pub(crate) enum Target {
    /// A table or view in the session at this index.
    Table {
        session: usize,
        table: TableRef,
    },
    Dashboard {
        session: usize,
        dashboard: Dashboard,
    },
    /// A history entry, opened as a query in the session at this index.
    Query {
        session: usize,
        sql: String,
    },
    /// Back up the session at this index to a file.
    Backup {
        session: usize,
    },
    /// Restore a backup file into the session at this index.
    Restore {
        session: usize,
    },
    Connect(SavedConnection),
}

/// One searchable entry.
#[derive(Clone)]
pub(crate) struct Item {
    pub(crate) icon: &'static str,
    pub(crate) label: String,
    pub(crate) source: String,
    pub(crate) mono: bool,
    pub(crate) target: Target,
    lower: String,
}

impl Item {
    pub(crate) fn new(
        icon: &'static str,
        label: String,
        source: String,
        mono: bool,
        target: Target,
    ) -> Self {
        let lower = label.to_lowercase();
        Self {
            icon,
            label,
            source,
            mono,
            target,
            lower,
        }
    }
}

/// Builds the table/view item for an object.
pub(crate) fn object_item(session: usize, name: &str, kind: ObjectKind, table: TableRef) -> Item {
    let (icon, source) = match kind {
        ObjectKind::View | ObjectKind::MaterializedView => ("eye", format!("{name} · view")),
        _ => (
            "table",
            format!(
                "{name} · {}",
                table.schema.as_ref().map_or("main", |s| s.as_str())
            ),
        ),
    };
    Item::new(
        icon,
        table.name.to_string(),
        source,
        true,
        Target::Table { session, table },
    )
}

/// Label for a history entry: its first comment, else its first line.
pub(crate) fn history_label(sql: &str) -> String {
    let first = sql
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let text = first.strip_prefix("--").map_or(first, str::trim);
    let text: String = text.chars().take(60).collect();
    format!("Query · {text}")
}

/// `today`, `yesterday` or a date.
pub(crate) fn day_label(at: chrono::DateTime<Utc>) -> String {
    let days = (Local::now().date_naive() - at.with_timezone(&Local).date_naive()).num_days();
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        _ => at.with_timezone(&Local).format("%b %-d").to_string(),
    }
}

/// Matching items, best first: name starts with the query, then contains it; shorter names
/// first. Returns each item's match range for highlighting.
pub(crate) fn search(items: &[Item], query: &str) -> Vec<(usize, Option<Range<usize>>)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..items.len().min(MAX_RESULTS))
            .map(|i| (i, None))
            .collect();
    }
    let mut hits: Vec<(u8, usize, usize, Range<usize>)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let at = item.lower.find(&q)?;
            // Lower-casing can change byte lengths; only highlight when it did not.
            let range = if item.lower.len() == item.label.len() {
                at..at + q.len()
            } else {
                0..0
            };
            Some((u8::from(at != 0), item.label.len(), i, range))
        })
        .collect();
    hits.sort_unstable_by_key(|(rank, len, i, _)| (*rank, *len, *i));
    hits.truncate(MAX_RESULTS);
    hits.into_iter()
        .map(|(_, _, i, r)| (i, (!r.is_empty()).then_some(r)))
        .collect()
}

pub(crate) enum PaletteEvent {
    Open { target: Box<Target>, new_tab: bool },
    Dismiss,
}

pub(crate) struct Palette {
    items: Vec<Item>,
    results: Vec<(usize, Option<Range<usize>>)>,
    selected: usize,
    input: Entity<InputState>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Palette {
    pub(crate) fn new(items: Vec<Item>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search tables, views, dashboards, history…")
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, input, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::Change) {
                this.results = search(&this.items, &input.read(cx).value());
                this.selected = 0;
                cx.notify();
            }
        });
        let results = search(&items, "");
        Self {
            items,
            results,
            selected: 0,
            input,
            focus: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    fn open(&mut self, ix: usize, new_tab: bool, cx: &mut Context<Self>) {
        if let Some((item, _)) = self.results.get(ix) {
            cx.emit(PaletteEvent::Open {
                target: Box::new(self.items[*item].target.clone()),
                new_tab,
            });
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keys = &event.keystroke;
        match keys.key.as_str() {
            "escape" => cx.emit(PaletteEvent::Dismiss),
            "down" => self.selected = (self.selected + 1).min(self.results.len().saturating_sub(1)),
            "up" => self.selected = self.selected.saturating_sub(1),
            "enter" => {
                let new_tab = keys.modifiers.secondary(cx);
                self.open(self.selected, new_tab, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("Palette::render").entered();
        let mono = theme::mono(cx);
        let hint = |key: &'static str, text: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().text_color(c(theme::MUTED)).child(key))
                .child(text)
        };
        let rows: Vec<_> = self
            .results
            .iter()
            .enumerate()
            .map(|(row, (ix, range))| {
                let item = &self.items[*ix];
                let selected = row == self.selected;
                let label = StyledText::new(SharedString::from(item.label.clone()))
                    .with_highlights(range.clone().map(|r| {
                        (
                            r,
                            HighlightStyle {
                                color: Some(theme::hsla(theme::ACCENT_TEXT)),
                                font_weight: Some(FontWeight::SEMIBOLD),
                                ..HighlightStyle::default()
                            },
                        )
                    }));
                div()
                    .id(("palette-row", row))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(40.))
                    .px(px(12.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(c(theme::SELECTION)))
                    .hover(|d| d.bg(c(theme::SELECTION)))
                    .child(ui::icon(
                        item.icon,
                        14.,
                        if selected && item.mono {
                            theme::ACCENT_TEXT
                        } else {
                            theme::MUTED
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(13.5))
                            .when(item.mono, |d| {
                                d.font_family(mono.clone()).text_size(px(13.))
                            })
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(theme::MUTED))
                            .child(SharedString::from(item.source.clone())),
                    )
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                        let new_tab = e.modifiers().secondary(cx);
                        this.open(row, new_tab, cx)
                    }))
            })
            .collect();
        let empty = rows.is_empty();
        div()
            .id("palette")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::on_key))
            .occlude()
            .w(px(620.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(c(theme::BG))
            .border_1()
            .border_color(c(theme::LINE))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(52.))
                    .px(px(16.))
                    .border_b_1()
                    .border_color(c(theme::LINE))
                    .child(ui::icon("search", 16., theme::MUTED))
                    .child(div().flex_1().child(ui::input(&self.input, 16.)))
                    .child(
                        div()
                            .px(px(6.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(c(theme::LINE))
                            .text_size(px(11.))
                            .text_color(c(theme::MUTED))
                            .font_family(mono.clone())
                            .child("esc"),
                    ),
            )
            .child(
                div()
                    .id("palette-results")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .p(px(6.))
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(12.))
                                .text_size(px(13.))
                                .text_color(c(theme::FAINT))
                                .child("No matches"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .h(px(36.))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(c(theme::LINE))
                    .text_size(px(12.))
                    .text_color(c(theme::MUTED))
                    .child(hint("↑↓", "navigate"))
                    .child(hint("↵", "open"))
                    .child(hint("⌘↵", "open in new tab"))
                    .child(div().ml_auto().child("Searches all open connections")),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use types::{ObjectKind, TableRef};

    use super::{history_label, object_item, search};

    #[test]
    fn ranks_prefix_matches_first_and_is_fast() {
        let mut items: Vec<_> = (0..10_000)
            .map(|i| {
                object_item(
                    0,
                    "db",
                    ObjectKind::Table,
                    TableRef::new(Some("public".into()), format!("t{i}_order_x")),
                )
            })
            .collect();
        items.push(object_item(
            0,
            "db",
            ObjectKind::Table,
            TableRef::new(Some("public".into()), "order_items"),
        ));
        items.push(object_item(
            0,
            "db",
            ObjectKind::Table,
            TableRef::new(Some("public".into()), "orders"),
        ));
        let started = Instant::now();
        let hits = search(&items, "Order");
        assert!(
            started.elapsed().as_millis() < 16,
            "search took {:?}",
            started.elapsed()
        );
        assert_eq!(items[hits[0].0].label, "orders");
        assert_eq!(items[hits[1].0].label, "order_items");
        assert_eq!(hits[0].1, Some(0..5));
        assert_eq!(hits.len(), 50);
        assert_eq!(
            history_label("-- top customers by revenue\nSELECT 1"),
            "Query · top customers by revenue"
        );
    }
}
