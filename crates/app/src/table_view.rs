//! A table document: toolbar, grid and paging status bar.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{Context, Entity, FontWeight, SharedString, Task, Window, div, px};
use traits::Connection;
use types::{Error, RowsRequest, TableRef};

use crate::grid::Grid;
use crate::theme::{self, c};
use crate::{runtime, ui};

/// Rows per page in the table view.
const PAGE: u64 = 50;

/// Data and paging for one table.
pub(crate) struct TableView {
    conn: Arc<dyn Connection>,
    pub(crate) table: TableRef,
    grid: Entity<Grid>,
    page: u64,
    total: Option<u64>,
    elapsed: Option<Duration>,
    error: Option<String>,
    loading: Option<Task<()>>,
}

impl TableView {
    pub(crate) fn new(conn: Arc<dyn Connection>, table: TableRef, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            conn,
            table,
            grid: cx.new(|_| Grid::new()),
            page: 0,
            total: None,
            elapsed: None,
            error: None,
            loading: None,
        };
        view.reload(cx);
        view
    }

    fn request(&self) -> RowsRequest {
        RowsRequest {
            offset: self.page * PAGE,
            limit: PAGE,
            ..RowsRequest::default()
        }
    }

    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let table = self.table.clone();
        let request = self.request();
        let first_row = (self.page * PAGE) as usize + 1;
        self.loading = Some(runtime::spawn_db(
            cx,
            async move {
                let browse = conn.browse().ok_or_else(|| Error::unsupported("browse"))?;
                let started = Instant::now();
                let page = browse.rows(&table, &request).await?;
                let elapsed = started.elapsed();
                let total = browse.count(&table, &RowsRequest::default()).await?;
                Ok::<_, Error>((page, elapsed, total))
            },
            move |this, result, cx| {
                this.loading = None;
                match result {
                    Ok((page, elapsed, total)) => {
                        this.elapsed = Some(elapsed);
                        this.total = Some(total);
                        this.error = None;
                        this.grid.update(cx, |grid, cx| {
                            grid.set_data(page.columns, page.rows, first_row);
                            cx.notify();
                        });
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    fn go(&mut self, delta: i64, cx: &mut Context<Self>) {
        let last = self.total.map_or(0, |t| t.saturating_sub(1) / PAGE);
        let next = (self.page as i64 + delta).clamp(0, last as i64) as u64;
        if next != self.page {
            self.page = next;
            self.reload(cx);
        }
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.grid.read(cx).row_count() as u64;
        let from = self.page * PAGE + u64::from(shown > 0);
        let to = self.page * PAGE + shown;
        let sep = || div().text_color(c(theme::FAINT)).child("·");
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(14.))
            .px(px(12.))
            .py(px(8.))
            .h(px(45.))
            .border_t_1()
            .border_color(c(theme::LINE))
            .bg(c(theme::PANEL))
            .text_size(px(12.))
            .text_color(c(theme::MUTED))
            .child(SharedString::from(match self.total {
                Some(n) => format!("{} rows", ui::thousands(n)),
                None => "Loading…".into(),
            }))
            .child(sep())
            .child(SharedString::from(
                self.elapsed
                    .map_or(String::new(), |d| format!("{} ms", d.as_millis())),
            ))
            .child(sep())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        ui::icon_button("prev-page", "chevron-left", 22., 14.)
                            .on_click(cx.listener(|this, _, _, cx| this.go(-1, cx))),
                    )
                    .child(SharedString::from(format!("Rows {from}–{to}")))
                    .child(
                        ui::icon_button("next-page", "chevron-right", 22., 14.)
                            .on_click(cx.listener(|this, _, _, cx| this.go(1, cx))),
                    ),
            )
    }
}

impl Render for TableView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.error {
            Some(error) => div()
                .p(px(16.))
                .text_size(px(13.))
                .text_color(c(theme::DELETED_TEXT))
                .child(SharedString::from(error.clone()))
                .into_any_element(),
            None => self.grid.clone().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(render_toolbar())
            .child(div().flex_1().min_h_0().child(body))
            .child(self.render_status(cx))
    }
}

/// Data / Structure toggle. Filters and row actions arrive with M4.
fn render_toolbar() -> impl IntoElement {
    let segment = |label: &'static str, active: bool| {
        let base = div()
            .flex()
            .items_center()
            .h(px(26.))
            .px(px(12.))
            .rounded(px(5.))
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer();
        if active {
            base.bg(c(theme::RAISED))
                .text_color(c(theme::TEXT))
                .border_1()
                .border_color(c(theme::LINE))
        } else {
            base.text_color(c(theme::MUTED))
        }
        .child(label)
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(8.))
        .border_b_1()
        .border_color(c(theme::LINE))
        .child(
            div()
                .flex()
                .gap(px(2.))
                .p(px(2.))
                .rounded(px(7.))
                .bg(c(theme::BG))
                .border_1()
                .border_color(c(theme::LINE))
                .child(segment("Data", true))
                .child(segment("Structure", false)),
        )
}
