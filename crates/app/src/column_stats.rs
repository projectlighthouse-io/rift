//! Column statistics popover, opened from a table's column header. One `GROUP BY` on the
//! whole table through `ColumnStats`.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{Context, EventEmitter, FontWeight, SharedString, Task, Window, div, px};
use traits::Connection;
use types::{Column, ColumnStatsReport, Error, TableRef, Value};

use crate::theme::{self, c};
use crate::{runtime, ui};

const TOP: u32 = 8;

/// What the popover asks its table view to do.
pub(crate) enum StatsAction {
    Close,
    Filter(Value),
    Chart,
}

pub(crate) struct ColumnStatsView {
    column: Column,
    report: Option<Result<ColumnStatsReport, String>>,
    _task: Task<()>,
}

impl EventEmitter<StatsAction> for ColumnStatsView {}

impl ColumnStatsView {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        table: TableRef,
        column: Column,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = column.name.clone();
        let task = runtime::spawn_db(
            cx,
            async move {
                match conn.column_stats() {
                    Some(s) => s.column_stats(&table, &name, TOP).await,
                    None => Err(Error::unsupported("column stats")),
                }
            },
            |this: &mut Self, result, cx| {
                this.report = Some(result.map_err(|e| e.to_string()));
                cx.notify();
            },
        );
        Self {
            column,
            report: None,
            _task: task,
        }
    }
}

fn stat(label: &'static str, value: String, mono: SharedString) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(c(theme::MUTED))
                .child(label),
        )
        .child(
            div()
                .font_family(mono)
                .text_size(px(12.5))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(value)),
        )
}

impl Render for ColumnStatsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let body = match &self.report {
            None => div()
                .text_size(px(12.5))
                .text_color(c(theme::FAINT))
                .child("Computing…")
                .into_any_element(),
            Some(Err(e)) => div()
                .text_size(px(12.5))
                .text_color(c(theme::DELETED_TEXT))
                .child(SharedString::from(e.clone()))
                .into_any_element(),
            Some(Ok(r)) => {
                let pct = (r.nulls * 100).checked_div(r.rows).unwrap_or(0);
                let top = r
                    .top
                    .first()
                    .map_or_else(|| "—".to_owned(), |v| v.value.to_string());
                let max = r.top.first().map_or(1, |v| v.count.max(1));
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(stat("Rows", ui::thousands(r.rows), mono.clone()))
                            .child(stat(
                                "Nulls",
                                format!("{} · {pct}%", ui::thousands(r.nulls)),
                                mono.clone(),
                            ))
                            .child(stat("Distinct", ui::thousands(r.distinct), mono.clone()))
                            .child(stat("Top", top, mono.clone())),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(c(theme::MUTED))
                            .child("Values"),
                    )
                    .child(div().flex().flex_col().gap(px(2.)).children(
                        r.top.iter().enumerate().map(|(i, v)| {
                            let value = v.value.clone();
                            let share = v.count as f32 / max as f32;
                            div()
                                .id(("stat-value", i))
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .h(px(22.))
                                .px(px(4.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|d| d.bg(c(theme::SELECTION)))
                                .font_family(mono.clone())
                                .text_size(px(12.5))
                                .child(
                                    div()
                                        .w(px(96.))
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(SharedString::from(v.value.to_string())),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .h(px(8.))
                                        .rounded(px(2.))
                                        .bg(c(theme::SELECTION))
                                        .child(
                                            div()
                                                .h_full()
                                                .w(gpui::relative(share))
                                                .rounded(px(2.))
                                                .bg(c(theme::ACCENT)),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(56.))
                                        .flex()
                                        .justify_end()
                                        .text_color(c(theme::MUTED))
                                        .child(SharedString::from(ui::thousands(v.count))),
                                )
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(StatsAction::Filter(value.clone()))
                                }))
                        }),
                    ))
                    .into_any_element()
            }
        };
        let top = self
            .report
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|r| r.top.first())
            .map(|v| v.value.clone());
        div()
            .id("column-stats")
            .occlude()
            .font_family(theme::SERIF)
            .whitespace_normal()
            .w(px(372.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .rounded(px(8.))
            .bg(c(theme::BG))
            .border_1()
            .border_color(c(theme::LINE))
            .shadow_lg()
            .text_color(c(theme::TEXT))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(StatsAction::Close)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .font_family(mono.clone())
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(SharedString::from(self.column.name.to_string())),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(c(theme::FAINT))
                            .child(SharedString::from(self.column.type_name.clone())),
                    )
                    .child(
                        div()
                            .id("stats-close")
                            .ml_auto()
                            .cursor_pointer()
                            .child(ui::icon("close", 14., theme::MUTED))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(StatsAction::Close))),
                    ),
            )
            .child(body)
            .child(div().h(px(1.)).bg(c(theme::LINE)))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::ghost_button("stats-filter", "Filter by value")
                            .flex_row_reverse()
                            .child(ui::icon("filter", 13., theme::MUTED))
                            .when_some(top, |b, v| {
                                b.on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(StatsAction::Filter(v.clone()))
                                }))
                            }),
                    )
                    .child(
                        ui::ghost_button("stats-chart", "Chart this column")
                            .flex_row_reverse()
                            .text_color(c(theme::ACCENT_TEXT))
                            .child(ui::icon("chart", 13., theme::ACCENT_TEXT))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(StatsAction::Chart))),
                    ),
            )
            .child(
                div().text_size(px(12.)).text_color(c(theme::MUTED)).child(
                    "Computed with one GROUP BY on the whole table, not just the loaded page.",
                ),
            )
    }
}
