//! A streamed query result in a grid. The SQL editor (M6) builds on this.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::*;
use gpui::{Context, Entity, SharedString, Task, Window, div, px};
use tokio_util::sync::CancellationToken;
use traits::{Connection, StatementOutput};
use types::{Column, Error, Row};

use crate::grid::Grid;
use crate::theme::{self, c};
use crate::{runtime, ui};

enum Event {
    Columns(Vec<Column>),
    Rows(Vec<Row>),
    Affected(u64),
    Done(Result<(), Error>),
}

/// Runs one statement and shows its rows as they stream in.
pub(crate) struct ResultView {
    pub(crate) title: SharedString,
    grid: Entity<Grid>,
    cancel: CancellationToken,
    started: Instant,
    elapsed: Option<Duration>,
    affected: Option<u64>,
    error: Option<String>,
    _pump: Task<()>,
}

impl ResultView {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        title: SharedString,
        sql: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::unbounded::<Event>();
        let token = cancel.clone();
        cx.global::<runtime::Io>().0.clone().spawn_task(async move {
            let send = |e| {
                let _ = tx.unbounded_send(e);
            };
            let Some(query) = conn.query() else {
                send(Event::Done(Err(Error::unsupported("query"))));
                return;
            };
            match query.execute(&sql, token).await {
                Ok(StatementOutput::Rows(mut stream)) => {
                    send(Event::Columns(stream.columns));
                    while let Some(page) = stream.pages.next().await {
                        match page {
                            Ok(rows) => send(Event::Rows(rows)),
                            Err(e) => return send(Event::Done(Err(e))),
                        }
                    }
                    send(Event::Done(Ok(())));
                }
                Ok(StatementOutput::Affected { rows }) => {
                    send(Event::Affected(rows));
                    send(Event::Done(Ok(())));
                }
                Ok(_) => send(Event::Done(Ok(()))),
                Err(e) => send(Event::Done(Err(e))),
            }
        });
        let pump = cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |view, cx| view.apply(event, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            title,
            grid: cx.new(|_| Grid::new()),
            cancel,
            started: Instant::now(),
            elapsed: None,
            affected: None,
            error: None,
            _pump: pump,
        }
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Columns(columns) => {
                self.grid
                    .update(cx, |g, _| g.set_data(columns, Vec::new(), 1));
            }
            Event::Rows(rows) => self.grid.update(cx, |g, cx| {
                g.append_rows(rows);
                cx.notify();
            }),
            Event::Affected(n) => self.affected = Some(n),
            Event::Done(result) => {
                self.elapsed = Some(self.started.elapsed());
                self.error = result.err().map(|e| e.to_string());
            }
        }
        cx.notify();
    }

    /// Stops the running query.
    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for ResultView {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Render for ResultView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.grid.read(cx).row_count() as u64;
        let status = match (&self.error, self.elapsed, self.affected) {
            (Some(e), _, _) => e.clone(),
            (None, Some(d), Some(n)) => format!(
                "Query OK · {} rows affected · {} ms",
                ui::thousands(n),
                d.as_millis()
            ),
            (None, Some(d), None) => format!(
                "Query OK · {} rows · {} ms",
                ui::thousands(rows),
                d.as_millis()
            ),
            (None, None, _) => format!("Running… {} rows", ui::thousands(rows)),
        };
        let color = if self.error.is_some() {
            theme::DELETED_TEXT
        } else {
            theme::MUTED
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(div().flex_1().min_h_0().child(self.grid.clone()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(45.))
                    .px(px(12.))
                    .border_t_1()
                    .border_color(c(theme::LINE))
                    .bg(c(theme::PANEL))
                    .text_size(px(12.))
                    .text_color(c(color))
                    .child(SharedString::from(status)),
            )
    }
}
