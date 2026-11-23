//! Virtualized data grid: header with name and type, row numbers, tabular-figure cells, `NULL` in faint
//! italics, numbers right-aligned, edit states, and inline editing. Only visible rows are built
//! (`uniform_list`).

use gpui::prelude::*;
use gpui::{
    BoxShadow, Context, Entity, EventEmitter, FontFeatures, FontWeight, KeyDownEvent, MouseButton,
    SharedString, Subscription, UniformListScrollHandle, Window, anchored, deferred, div, px,
    uniform_list,
};
use gpui_component::input::{InputEvent, InputState};
use types::{Column, Row, Value};

use crate::theme::{self, c};
use crate::ui;

/// Row height including its bottom border.
pub(crate) const ROW_HEIGHT: f32 = 41.0;
const HEADER_HEIGHT: f32 = 41.0;
const ROW_NUMBER_WIDTH: f32 = 64.0;
/// Approximate Inter advance at 12.5px / 12px / 11px, rounded up so widths rarely clip.
// ponytail: char-count estimate; measure with the text system if columns look off.
const CELL_CHAR: f32 = 7.55;
const HEADER_CHAR: f32 = 7.25;
const TYPE_CHAR: f32 = 6.65;
const MAX_WIDTH: f32 = 280.0;
const MAX_CELL_CHARS: usize = 200;

/// State of a whole row.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RowMark {
    #[default]
    None,
    /// Not saved yet; shown green with `+`.
    New,
    /// Will be deleted on save; red, struck through, `−`.
    Deleted,
}

/// A row as the grid shows it.
pub(crate) struct GridRow {
    pub(crate) values: Vec<Value>,
    pub(crate) mark: RowMark,
    /// Per cell: changed but not saved (yellow).
    pub(crate) edited: Vec<bool>,
}

impl GridRow {
    pub(crate) fn plain(row: Row) -> Self {
        let edited = vec![false; row.values.len()];
        Self {
            values: row.values,
            mark: RowMark::None,
            edited,
        }
    }
}

/// What the grid tells its owner.
pub(crate) enum GridEvent {
    /// A cell was clicked.
    Selected { row: usize },
    /// A column header was clicked.
    Header { col: usize },
    /// Inline edit committed with this text.
    Edited {
        row: usize,
        col: usize,
        text: String,
    },
}

struct Editing {
    row: usize,
    col: usize,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// Grid state for one result.
pub(crate) struct Grid {
    columns: Vec<Column>,
    widths: Vec<f32>,
    numeric: Vec<bool>,
    rows: Vec<GridRow>,
    first_row_number: usize,
    selected_row: Option<usize>,
    focused_cell: Option<(usize, usize)>,
    editable: bool,
    editing: Option<Editing>,
    /// A view anchored under a column header (column stats).
    popover: Option<(usize, gpui::AnyView)>,
    scroll: UniformListScrollHandle,
    /// Dev check (`RIFT_DEV_AUTOSCROLL=1`): scrolls through every row, one step per frame.
    autoscroll: Option<usize>,
}

impl EventEmitter<GridEvent> for Grid {}

impl Grid {
    pub(crate) fn new() -> Self {
        Self {
            columns: Vec::new(),
            widths: Vec::new(),
            numeric: Vec::new(),
            rows: Vec::new(),
            first_row_number: 1,
            selected_row: None,
            focused_cell: None,
            editable: false,
            editing: None,
            popover: None,
            scroll: UniformListScrollHandle::new(),
            autoscroll: std::env::var_os("RIFT_DEV_AUTOSCROLL").map(|_| 0),
        }
    }

    /// Shows `view` under column `col`'s header, or nothing.
    pub(crate) fn set_popover(&mut self, popover: Option<(usize, gpui::AnyView)>) {
        self.popover = popover;
    }

    /// Allows double-click editing.
    pub(crate) fn set_editable(&mut self, editable: bool) {
        self.editable = editable;
    }

    /// Replaces the data with plain rows. `first_row_number` is shown for row 0 (paging).
    pub(crate) fn set_data(
        &mut self,
        columns: Vec<Column>,
        rows: Vec<Row>,
        first_row_number: usize,
    ) {
        self.set_rows(
            columns,
            rows.into_iter().map(GridRow::plain).collect(),
            first_row_number,
        );
        self.selected_row = None;
        self.focused_cell = None;
    }

    /// Replaces rows, keeping column widths and selection when the columns are unchanged.
    pub(crate) fn set_rows(
        &mut self,
        columns: Vec<Column>,
        rows: Vec<GridRow>,
        first_row_number: usize,
    ) {
        let same_columns = columns == self.columns;
        self.columns = columns;
        self.rows = rows;
        self.first_row_number = first_row_number;
        if !same_columns || self.widths.is_empty() {
            self.measure();
            self.selected_row = None;
            self.focused_cell = None;
        }
        if self.selected_row.is_some_and(|r| r >= self.rows.len()) {
            self.selected_row = None;
            self.focused_cell = None;
        }
    }

    /// Appends streamed rows, measuring on the first page.
    pub(crate) fn append_rows(&mut self, rows: Vec<Row>) {
        let first = self.rows.is_empty();
        self.rows.extend(rows.into_iter().map(GridRow::plain));
        if first {
            self.measure();
        }
    }

    /// Columns shown.
    pub(crate) fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Cell values of every row, for copy and export.
    pub(crate) fn values(&self) -> impl Iterator<Item = &[Value]> {
        self.rows.iter().map(|r| r.values.as_slice())
    }

    /// Selects a row (and its first cell) and scrolls to it.
    pub(crate) fn select(&mut self, row: usize, cx: &mut Context<Self>) {
        self.selected_row = Some(row);
        self.focused_cell = Some((row, 0));
        self.scroll
            .scroll_to_item(row, gpui::ScrollStrategy::Center);
        cx.notify();
    }

    fn measure(&mut self) {
        let sample = &self.rows[..self.rows.len().min(200)];
        self.numeric = (0..self.columns.len())
            .map(|i| {
                sample
                    .iter()
                    .filter_map(|r| r.values.get(i))
                    .find(|v| !v.is_null())
                    .is_some_and(Value::is_numeric)
            })
            .collect();
        self.widths = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let mut header = col.name.as_str().len() as f32 * HEADER_CHAR
                    + 4.0
                    + col.type_name.len() as f32 * TYPE_CHAR;
                if col.primary_key || col.references.is_some() {
                    header += 16.0;
                }
                let widest = sample
                    .iter()
                    .filter_map(|r| r.values.get(i))
                    .map(|v| cell_text(v).chars().count())
                    .max()
                    .unwrap_or(0) as f32
                    * CELL_CHAR;
                // Header cells pad 14px a side, data cells 10px (the design).
                (header + 29.0).max(widest + 21.0).clamp(48.0, MAX_WIDTH)
            })
            .collect();
    }

    fn total_width(&self) -> f32 {
        ROW_NUMBER_WIDTH + self.widths.iter().sum::<f32>()
    }

    fn start_edit(&mut self, row: usize, col: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable
            || self
                .rows
                .get(row)
                .is_none_or(|r| r.mark == RowMark::Deleted)
        {
            return;
        }
        let text = match &self.rows[row].values[col] {
            Value::Null => String::new(),
            v => v.to_string(),
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        // Focus after this mouse-down finishes bubbling, or a focusable ancestor takes it back.
        let focus_target = input.clone();
        cx.defer_in(window, move |_, window, cx| {
            focus_target.update(cx, |s, cx| s.focus(window, cx));
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit_edit(cx);
            }
        });
        self.editing = Some(Editing {
            row,
            col,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(editing) = self.editing.take() {
            let text = editing.input.read(cx).value().to_string();
            cx.emit(GridEvent::Edited {
                row: editing.row,
                col: editing.col,
                text,
            });
            cx.notify();
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.editing.is_some() {
            self.editing = None;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cell = |w: f32| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .w(px(w))
                .h(px(HEADER_HEIGHT))
                .px(px(14.))
                .border_b_1()
                .border_color(c(theme::LINE))
                .bg(c(theme::PANEL))
                .overflow_hidden()
                .whitespace_nowrap()
        };
        div()
            .flex()
            .flex_none()
            .child(cell(ROW_NUMBER_WIDTH))
            .children(
                self.columns
                    .iter()
                    .zip(&self.widths)
                    .enumerate()
                    .map(|(i, (col, w))| {
                        let open = self
                            .popover
                            .as_ref()
                            .filter(|(c, _)| *c == i)
                            .map(|(_, v)| v.clone());
                        let marker = if col.primary_key {
                            Some("key")
                        } else if col.references.is_some() {
                            Some("link")
                        } else {
                            None
                        };
                        cell(*w)
                            .id(("grid-header", i))
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |_, _, _, cx| {
                                    cx.emit(GridEvent::Header { col: i })
                                }),
                            )
                            .gap(px(4.))
                            .children(marker.map(|m| ui::icon(m, 12., theme::FAINT)))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(c(if open.is_some() {
                                        theme::ACCENT_TEXT
                                    } else {
                                        theme::TEXT
                                    }))
                                    .when(open.is_some(), |d| d.underline())
                                    .child(SharedString::from(col.name.to_string())),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(c(theme::FAINT))
                                    .child(SharedString::from(col.type_name.clone())),
                            )
                            .when_some(open, |d, view| {
                                d.relative().child(
                                    div().absolute().left_0().top(px(HEADER_HEIGHT + 4.)).child(
                                        deferred(anchored().snap_to_window().child(view))
                                            .with_priority(1),
                                    ),
                                )
                            })
                    }),
            )
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui::Div {
        let row = &self.rows[ix];
        let selected = self.selected_row == Some(ix);
        let (row_bg, row_fg, number_fg, label): (u32, u32, u32, SharedString) = match row.mark {
            RowMark::New => (theme::NEW_BG, theme::NEW_TEXT, theme::NEW_TEXT, "+".into()),
            RowMark::Deleted => (
                theme::DELETED_BG,
                theme::DELETED_TEXT,
                theme::DELETED_TEXT,
                "−".into(),
            ),
            RowMark::None => (
                if selected {
                    theme::SELECTION
                } else if ix % 2 == 1 {
                    theme::STRIPE
                } else {
                    theme::BG
                },
                theme::TEXT,
                theme::FAINT,
                (self.first_row_number + ix).to_string().into(),
            ),
        };
        div()
            .flex()
            .h(px(ROW_HEIGHT))
            .w(px(self.total_width()))
            .bg(c(row_bg))
            .border_b_1()
            .border_color(c(theme::LINE))
            // Tabular figures so numbers and dates line up down a column.
            .font_features(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .w(px(ROW_NUMBER_WIDTH))
                    .h_full()
                    .px(px(10.))
                    .text_size(px(11.5))
                    .text_color(c(number_fg))
                    .child(label),
            )
            .children(row.values.iter().enumerate().map(|(col, value)| {
                let width = self.widths.get(col).copied().unwrap_or(120.);
                let mut cell = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .w(px(width))
                    .h_full()
                    .px(px(10.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.5))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                            if this
                                .editing
                                .as_ref()
                                .is_some_and(|e| (e.row, e.col) != (ix, col))
                            {
                                this.commit_edit(cx);
                            }
                            this.selected_row = Some(ix);
                            this.focused_cell = Some((ix, col));
                            cx.emit(GridEvent::Selected { row: ix });
                            if event.click_count == 2 {
                                this.start_edit(ix, col, window, cx);
                            }
                            cx.notify();
                        }),
                    );
                if let Some(editing) = self
                    .editing
                    .as_ref()
                    .filter(|e| (e.row, e.col) == (ix, col))
                {
                    return cell.child(
                        pill(theme::BG, focus_ring())
                            .flex_1()
                            .py(px(0.))
                            .px(px(0.))
                            .child(ui::input(&editing.input, 12.5)),
                    );
                }
                let numeric = self.numeric.get(col).copied().unwrap_or(false);
                if numeric {
                    cell = cell.justify_end();
                }
                let edited =
                    row.mark == RowMark::None && row.edited.get(col).copied().unwrap_or(false);
                let focused = self.focused_cell == Some((ix, col));
                if row.mark == RowMark::Deleted {
                    cell = cell.line_through();
                }
                let fg = if edited { theme::EDITED_TEXT } else { row_fg };
                let content = if value.is_null() {
                    let null_fg = if row.mark == RowMark::None && !edited {
                        theme::FAINT
                    } else {
                        fg
                    };
                    div().text_color(c(null_fg)).italic().child("NULL")
                } else {
                    div()
                        .text_color(c(fg))
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(SharedString::from(cell_text(value)))
                };
                // Edited and focused values sit in a soft pill; the cell itself has no box.
                if focused {
                    cell.child(pill(theme::BG, focus_ring()).child(content))
                } else if edited {
                    cell.child(pill(theme::EDITED_BG, edited_ring()).child(content))
                } else {
                    cell.child(content)
                }
            }))
    }
}

/// The value pill: radius 7, 4px × 8px padding, pulled 8px out so its text stays aligned.
fn pill(bg: u32, ring: Vec<BoxShadow>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .min_w_0()
        .mx(px(-8.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(7.))
        .bg(c(bg))
        .shadow(ring)
}

/// `0 0 0 1.5px #e57368, 0 0 0 4px rgba(229,115,104,0.15)`.
fn focus_ring() -> Vec<BoxShadow> {
    let mut halo = theme::hsla(theme::ACCENT);
    halo.a = 0.15;
    vec![ring(theme::hsla(theme::ACCENT), 1.5), ring(halo, 4.)]
}

/// `inset 0 0 0 1px rgba(154,103,0,0.18)`.
fn edited_ring() -> Vec<BoxShadow> {
    let mut color = theme::hsla(theme::EDITED_TEXT);
    color.a = 0.18;
    vec![BoxShadow {
        inset: true,
        ..ring(color, 1.)
    }]
}

fn ring(color: gpui::Hsla, spread: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(spread),
        inset: false,
    }
}

/// Display text for a cell: one line, capped length.
fn cell_text(value: &Value) -> String {
    let text = match value {
        Value::Decimal(_) | Value::Float(_) => group_digits(&value.to_string()),
        _ => value.to_string(),
    };
    text.chars()
        .take(MAX_CELL_CHARS)
        .map(|ch| if ch == '\n' { ' ' } else { ch })
        .collect()
}

/// `1842.50` → `1,842.50`; leaves anything that is not a plain number alone.
fn group_digits(text: &str) -> String {
    let (sign, rest) = text.strip_prefix('-').map_or(("", text), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    if int.len() <= 3 || !int.bytes().all(|b| b.is_ascii_digit()) {
        return text.to_owned();
    }
    let grouped = crate::ui::thousands(int.parse().unwrap_or_default());
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

impl Render for Grid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("Grid::render").entered();
        let count = self.rows.len();
        if let Some(pos) = self.autoscroll.filter(|_| count > 10_000) {
            let next = pos + 97;
            if next < count {
                self.scroll.scroll_to_item(next, gpui::ScrollStrategy::Top);
                self.autoscroll = Some(next);
                window.request_animation_frame();
            } else {
                tracing::info!(rows = count, "autoscroll finished");
                self.autoscroll = None;
            }
        }
        let width = self.total_width();
        div()
            .id("grid-h")
            .size_full()
            .overflow_x_scroll()
            // Without this GPUI turns vertical wheel deltas into horizontal scroll here (this
            // div only overflows on x), so scrolling rows also drifts sideways.
            .restrict_scroll_to_axis()
            .capture_key_down(cx.listener(Self::on_key))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(width))
                    .h_full()
                    .child(self.render_header(cx))
                    .child(
                        uniform_list(
                            "grid-rows",
                            count,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                let _span =
                                    tracing::debug_span!("Grid::rows", n = range.len()).entered();
                                range.map(|ix| this.render_row(ix, cx)).collect()
                            }),
                        )
                        .flex_1()
                        // The mirror case: a sideways swipe must not scroll rows.
                        .map(|mut list| {
                            list.style().restrict_scroll_to_axis = Some(true);
                            list
                        })
                        .track_scroll(&self.scroll),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn groups_decimal_digits() {
        assert_eq!(super::group_digits("1842.50"), "1,842.50");
        assert_eq!(super::group_digits("-12345"), "-12,345");
        assert_eq!(super::group_digits("99.99"), "99.99");
        assert_eq!(super::group_digits("NaN"), "NaN");
    }
}
