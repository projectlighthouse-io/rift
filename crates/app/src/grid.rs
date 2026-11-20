//! Virtualized data grid: header with name and type, row numbers, mono cells, `NULL` in faint
//! italics, numbers right-aligned. Only visible rows are built (`uniform_list`).

use gpui::prelude::*;
use gpui::{
    Context, FontWeight, MouseButton, SharedString, UniformListScrollHandle, Window, div, px,
    uniform_list,
};
use types::{Column, Row, Value};

use crate::theme::{self, c};
use crate::ui;

/// Row height including its bottom border.
pub(crate) const ROW_HEIGHT: f32 = 31.0;
const HEADER_HEIGHT: f32 = 33.0;
const ROW_NUMBER_WIDTH: f32 = 64.0;
/// Approximate advance of the mono font at 12.5px / 12px / 11px.
const CELL_CHAR: f32 = 7.55;
const HEADER_CHAR: f32 = 7.25;
const TYPE_CHAR: f32 = 6.65;
const MAX_WIDTH: f32 = 280.0;
const MAX_CELL_CHARS: usize = 200;

/// Grid state for one result.
pub(crate) struct Grid {
    columns: Vec<Column>,
    widths: Vec<f32>,
    numeric: Vec<bool>,
    rows: Vec<Row>,
    first_row_number: usize,
    selected_row: Option<usize>,
    focused_cell: Option<(usize, usize)>,
    scroll: UniformListScrollHandle,
    /// Dev check (`RIFT_DEV_AUTOSCROLL=1`): scrolls through every row, one step per frame.
    autoscroll: Option<usize>,
}

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
            scroll: UniformListScrollHandle::new(),
            autoscroll: std::env::var_os("RIFT_DEV_AUTOSCROLL").map(|_| 0),
        }
    }

    /// Replaces the data. `first_row_number` is shown for row 0 (paging).
    pub(crate) fn set_data(
        &mut self,
        columns: Vec<Column>,
        rows: Vec<Row>,
        first_row_number: usize,
    ) {
        self.columns = columns;
        self.rows = rows;
        self.first_row_number = first_row_number;
        self.selected_row = None;
        self.focused_cell = None;
        self.measure();
    }

    /// Appends streamed rows, re-measuring only for the first page.
    pub(crate) fn append_rows(&mut self, rows: Vec<Row>) {
        let first = self.rows.is_empty();
        self.rows.extend(rows);
        if first {
            self.measure();
        }
    }

    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn measure(&mut self) {
        let sample = &self.rows[..self.rows.len().min(200)];
        self.numeric = (0..self.columns.len())
            .map(|i| {
                sample
                    .iter()
                    .map(|r| &r.values[i])
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
                    .map(|r| cell_text(&r.values[i]).chars().count())
                    .max()
                    .unwrap_or(0) as f32
                    * CELL_CHAR;
                (header.max(widest) + 21.0).clamp(48.0, MAX_WIDTH)
            })
            .collect();
    }

    fn total_width(&self) -> f32 {
        ROW_NUMBER_WIDTH + self.widths.iter().sum::<f32>()
    }

    fn render_header(&self, mono: SharedString) -> impl IntoElement {
        let cell = |w: f32| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .w(px(w))
                .h(px(HEADER_HEIGHT))
                .px(px(10.))
                .border_r_1()
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
            .children(self.columns.iter().zip(&self.widths).map(|(col, w)| {
                let marker = if col.primary_key {
                    Some("key")
                } else if col.references.is_some() {
                    Some("link")
                } else {
                    None
                };
                cell(*w)
                    .gap(px(4.))
                    .font_family(mono.clone())
                    .children(marker.map(|m| ui::icon(m, 12., theme::FAINT)))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(theme::TEXT))
                            .child(SharedString::from(col.name.to_string())),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(theme::FAINT))
                            .child(SharedString::from(col.type_name.clone())),
                    )
            }))
    }

    fn render_row(&self, ix: usize, mono: SharedString, cx: &mut Context<Self>) -> gpui::Div {
        let row = &self.rows[ix];
        let selected = self.selected_row == Some(ix);
        let bg = if selected {
            theme::SELECTION
        } else {
            theme::BG
        };
        div()
            .flex()
            .h(px(ROW_HEIGHT))
            .w(px(self.total_width()))
            .bg(c(bg))
            .border_b_1()
            .border_color(c(theme::LINE))
            .font_family(mono)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .w(px(ROW_NUMBER_WIDTH))
                    .h_full()
                    .px(px(10.))
                    .border_r_1()
                    .border_color(c(theme::LINE))
                    .text_size(px(11.5))
                    .text_color(c(theme::FAINT))
                    .child(SharedString::from((self.first_row_number + ix).to_string())),
            )
            .children(row.values.iter().enumerate().map(|(col, value)| {
                let focused = self.focused_cell == Some((ix, col));
                let mut cell = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .w(px(self.widths[col]))
                    .h_full()
                    .px(px(10.))
                    .border_r_1()
                    .border_color(c(theme::LINE))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.5))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.selected_row = Some(ix);
                            this.focused_cell = Some((ix, col));
                            cx.notify();
                        }),
                    );
                if self.numeric.get(col).copied().unwrap_or(false) {
                    cell = cell.justify_end();
                }
                if focused {
                    cell = cell.border_2().border_color(c(theme::ACCENT));
                }
                if value.is_null() {
                    cell.text_color(c(theme::FAINT)).italic().child("NULL")
                } else {
                    cell.text_color(c(theme::TEXT))
                        .text_ellipsis()
                        .child(SharedString::from(cell_text(value)))
                }
            }))
    }
}

/// Display text for a cell: one line, capped length.
fn cell_text(value: &Value) -> String {
    let text = value.to_string();
    let line: String = text
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|ch| if ch == '\n' { ' ' } else { ch })
        .collect();
    line
}

impl Render for Grid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
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
        let row_mono = mono.clone();
        div().id("grid-h").size_full().overflow_x_scroll().child(
            div()
                .flex()
                .flex_col()
                .w(px(width))
                .h_full()
                .child(self.render_header(mono))
                .child(
                    uniform_list(
                        "grid-rows",
                        count,
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|ix| this.render_row(ix, row_mono.clone(), cx))
                                .collect()
                        }),
                    )
                    .flex_1()
                    .track_scroll(&self.scroll),
                ),
        )
    }
}
