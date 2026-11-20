//! SQL editor: highlighted editor with the current statement marked, run current / all,
//! explain, format, per-statement result tabs, copy/export, history and autocomplete.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::*;
use gpui::{
    ClipboardItem, Context, Entity, FontWeight, KeyDownEvent, SharedString, Subscription, Task,
    Window, anchored, deferred, div, px,
};
use gpui_component::input::{
    Editor, EditorState, InputEvent, RangeDecoration, RangeDecorationCollection,
    RangeDecorationStyle,
};
use lsp_types::CompletionItemKind;
use tokio_util::sync::CancellationToken;
use traits::{Connection, StatementOutput};
use types::{Column, ConnectionId, Error, HistoryEntry, ObjectKind, Row, Tag};

use crate::chart_panel::{ChartPanel, PinChart};
use crate::grid::Grid;
use crate::sql_syntax::{self, SchemaNames, SqlCompletions};
use crate::theme::{self, c};
use crate::{export, runtime, state, ui};

const EDITOR_HEIGHT: f32 = 330.0;
const CURRENT_STATEMENT: u32 = 0xf6f4f1;
/// `active_tab` value of the Chart tab.
const CHART_TAB: usize = usize::MAX;

enum RunEvent {
    Columns {
        columns: Vec<Column>,
        sql: String,
    },
    Rows {
        rows: Vec<Row>,
    },
    Done {
        ix: usize,
        rows: u64,
        affected: Option<u64>,
        ms: u128,
    },
    Failed {
        ix: usize,
        message: String,
    },
    Plan {
        text: String,
    },
    Finished,
}

struct ResultTab {
    grid: Entity<Grid>,
    rows: u64,
    sql: String,
}

struct Message {
    text: String,
    error: bool,
}

/// Outcome of the last run, for the status bar.
struct Summary {
    ok: bool,
    rows: u64,
    ms: u128,
    text: String,
}

/// A chart was pinned to a dashboard.
pub(crate) struct Pinned;

impl gpui::EventEmitter<Pinned> for SqlEditor {}

/// One query tab.
pub(crate) struct SqlEditor {
    conn: Arc<dyn Connection>,
    connection_id: ConnectionId,
    label: SharedString,
    tag: Tag,
    pub(crate) title: SharedString,
    editor: Entity<EditorState>,
    current: RangeDecorationCollection,
    marked: Option<std::ops::Range<usize>>,
    results: Vec<ResultTab>,
    chart: Option<Entity<ChartPanel>>,
    messages: Vec<Message>,
    /// Index into `results`; `results.len()` means the Messages tab.
    active_tab: usize,
    running: Option<CancellationToken>,
    summary: Option<Summary>,
    history: Option<Vec<HistoryEntry>>,
    copy_menu: bool,
    _pump: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl SqlEditor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        connection_id: ConnectionId,
        label: SharedString,
        tag: Tag,
        title: SharedString,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("sql")
                .line_number(true)
                .default_value(sql);
            state.set_highlighter_factory(sql_syntax::highlighter_factory(), cx);
            state
        });
        let names = SchemaNames::default();
        editor.update(cx, |state, _| {
            state.lsp_mut().completion_provider = Some(Rc::new(SqlCompletions {
                names: names.clone(),
            }));
        });
        let current = editor.update(cx, |state, cx| {
            state.create_range_decorations_collection(Vec::new(), cx)
        });
        let observe = cx.observe(&editor, |this, _, cx| this.mark_current_statement(cx));
        let changed = cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.mark_current_statement(cx);
            }
        });
        editor.update(cx, |s, cx| s.focus(window, cx));
        let mut view = Self {
            conn,
            connection_id,
            label,
            tag,
            title,
            editor,
            current,
            marked: None,
            results: Vec::new(),
            chart: None,
            messages: Vec::new(),
            active_tab: 0,
            running: None,
            summary: None,
            history: None,
            copy_menu: false,
            _pump: None,
            _subscriptions: vec![observe, changed],
        };
        view.load_schema_names(names, cx);
        view.mark_current_statement(cx);
        view
    }

    /// Fills autocomplete with tables and their columns (first 200 tables).
    fn load_schema_names(&mut self, names: SchemaNames, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        runtime::spawn_db(
            cx,
            async move {
                let Some(browse) = conn.browse() else { return };
                let Ok(Some(schema)) = browse.default_schema().await else {
                    return;
                };
                let Ok(objects) = browse.objects(&schema).await else {
                    return;
                };
                let mut out: Vec<(String, CompletionItemKind)> = Vec::new();
                for object in objects.iter().take(200) {
                    let kind = match object.kind {
                        ObjectKind::Function => CompletionItemKind::FUNCTION,
                        _ => CompletionItemKind::STRUCT,
                    };
                    out.push((object.table.name.to_string(), kind));
                    if matches!(object.kind, ObjectKind::Table | ObjectKind::View)
                        && let Some(describe) = conn.describe()
                        && let Ok(desc) = describe.describe(&object.table).await
                    {
                        out.extend(
                            desc.columns
                                .into_iter()
                                .map(|c| (c.name.to_string(), CompletionItemKind::FIELD)),
                        );
                    }
                }
                names.set(out);
            },
            |_, (), _| {},
        )
        .detach();
    }

    fn text(&self, cx: &gpui::App) -> String {
        self.editor.read(cx).value().to_string()
    }

    fn current_range(&self, cx: &gpui::App) -> Option<std::ops::Range<usize>> {
        let state = self.editor.read(cx);
        dialect::split::statement_at(&state.value(), state.cursor())
    }

    fn mark_current_statement(&mut self, cx: &mut Context<Self>) {
        let range = self.current_range(cx);
        // Setting decorations notifies the editor, which re-runs this observer: only touch
        // them when the statement actually moved.
        if range == self.marked {
            return;
        }
        self.marked = range.clone();
        let decorations = range
            .map(|r| {
                RangeDecoration::new(r)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(theme::hsla(CURRENT_STATEMENT))
            })
            .into_iter()
            .collect();
        self.current.set(decorations, cx);
        cx.notify();
    }

    fn run_current(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if let Some(range) = self.current_range(cx) {
            self.run(vec![text[range].to_owned()], cx);
        }
    }

    fn run_all(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let statements: Vec<String> = dialect::split::statements(&text)
            .into_iter()
            .map(|r| text[r].to_owned())
            .collect();
        if !statements.is_empty() {
            self.run(statements, cx);
        }
    }

    fn explain(&mut self, cx: &mut Context<Self>) {
        let Some(range) = self.current_range(cx) else {
            return;
        };
        let sql = self.text(cx)[range].to_owned();
        let conn = self.conn.clone();
        self.start(cx, move |tx, _| async move {
            let result = match conn.explain() {
                Some(e) => e.explain(&sql).await,
                None => Err(Error::unsupported("explain")),
            };
            let _ = tx.unbounded_send(match result {
                Ok(plan) => RunEvent::Plan { text: plan.text },
                Err(e) => RunEvent::Failed {
                    ix: 0,
                    message: e.to_string(),
                },
            });
            let _ = tx.unbounded_send(RunEvent::Finished);
        });
    }

    fn format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let options = sqlformat::FormatOptions {
            uppercase: Some(true),
            ..sqlformat::FormatOptions::default()
        };
        let formatted = sqlformat::format(&text, &sqlformat::QueryParams::None, &options);
        self.editor
            .update(cx, |s, cx| s.set_value(formatted, window, cx));
    }

    /// Clears results and runs `job` on the I/O runtime, pumping its events into the view.
    fn start<F, Fut>(&mut self, cx: &mut Context<Self>, job: F)
    where
        F: FnOnce(mpsc::UnboundedSender<RunEvent>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if let Some(token) = self.running.take() {
            token.cancel();
        }
        let token = CancellationToken::new();
        self.running = Some(token.clone());
        self.results.clear();
        self.chart = None;
        self.messages.clear();
        self.active_tab = 0;
        self.summary = None;
        let (tx, mut rx) = mpsc::unbounded();
        cx.global::<runtime::Io>().0.spawn_task(job(tx, token));
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |view, cx| view.apply(event, cx)).is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn run(&mut self, statements: Vec<String>, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let history = state::history(cx);
        let connection = self.connection_id;
        self.start(cx, move |tx, token| async move {
            let send = |e| {
                let _ = tx.unbounded_send(e);
            };
            for (ix, sql) in statements.into_iter().enumerate() {
                let started = Instant::now();
                let ran_at = Utc::now();
                let outcome: Result<(u64, Option<u64>), Error> = async {
                    let query = conn.query().ok_or_else(|| Error::unsupported("query"))?;
                    match query.execute(&sql, token.clone()).await? {
                        StatementOutput::Rows(mut stream) => {
                            send(RunEvent::Columns {
                                columns: stream.columns,
                                sql: sql.clone(),
                            });
                            let mut count = 0u64;
                            while let Some(page) = stream.pages.next().await {
                                let rows = page?;
                                count += rows.len() as u64;
                                send(RunEvent::Rows { rows });
                            }
                            Ok((count, None))
                        }
                        StatementOutput::Affected { rows } => Ok((0, Some(rows))),
                        _ => Ok((0, None)),
                    }
                }
                .await;
                let ms = started.elapsed().as_millis();
                let entry = HistoryEntry {
                    connection,
                    sql: sql.clone(),
                    ran_at,
                    duration_ms: ms as u64,
                    rows: outcome.as_ref().ok().map(|(r, a)| a.unwrap_or(*r)),
                    error: outcome.as_ref().err().map(ToString::to_string),
                };
                if let Err(e) = history.append(&entry) {
                    tracing::warn!("saving history: {e}");
                }
                match outcome {
                    Ok((rows, affected)) => send(RunEvent::Done {
                        ix,
                        rows,
                        affected,
                        ms,
                    }),
                    Err(e) => {
                        send(RunEvent::Failed {
                            ix,
                            message: e.to_string(),
                        });
                        break;
                    }
                }
            }
            send(RunEvent::Finished);
        });
    }

    fn apply(&mut self, event: RunEvent, cx: &mut Context<Self>) {
        match event {
            RunEvent::Columns { columns, sql } => {
                let grid = cx.new(|_| Grid::new());
                grid.update(cx, |g, _| g.set_data(columns, Vec::new(), 1));
                self.results.push(ResultTab { grid, rows: 0, sql });
            }
            RunEvent::Rows { rows } => {
                if let Some(tab) = self.results.last_mut() {
                    tab.rows += rows.len() as u64;
                    tab.grid.update(cx, |g, cx| {
                        g.append_rows(rows);
                        cx.notify();
                    });
                }
            }
            RunEvent::Done {
                ix,
                rows,
                affected,
                ms,
            } => {
                let text = match affected {
                    Some(n) => format!(
                        "Statement {}: {} rows affected · {ms} ms",
                        ix + 1,
                        ui::thousands(n)
                    ),
                    None => format!(
                        "Statement {}: {} rows · {ms} ms",
                        ix + 1,
                        ui::thousands(rows)
                    ),
                };
                self.messages.push(Message { text, error: false });
                self.summary = Some(Summary {
                    ok: true,
                    rows: affected.unwrap_or(rows),
                    ms,
                    text: if affected.is_some() {
                        "rows affected".into()
                    } else {
                        "read-only result".into()
                    },
                });
                if self.results.is_empty() {
                    self.active_tab = 0;
                }
            }
            RunEvent::Failed { ix, message } => {
                self.messages.push(Message {
                    text: format!("Statement {}: {message}", ix + 1),
                    error: true,
                });
                self.summary = Some(Summary {
                    ok: false,
                    rows: 0,
                    ms: 0,
                    text: message,
                });
                self.active_tab = self.results.len();
            }
            RunEvent::Plan { text } => {
                self.messages.extend(text.lines().map(|l| Message {
                    text: l.to_owned(),
                    error: false,
                }));
                self.summary = Some(Summary {
                    ok: true,
                    rows: 0,
                    ms: 0,
                    text: "plan".into(),
                });
                self.active_tab = self.results.len();
            }
            RunEvent::Finished => {
                self.running = None;
                if self.results.is_empty() {
                    self.active_tab = 0;
                }
            }
        }
        cx.notify();
    }

    /// Stops the running query (Esc, closing the tab).
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(token) = self.running.take() {
            token.cancel();
            cx.notify();
        }
    }

    /// Shows the Chart tab for the last result, building the panel on first use.
    fn open_chart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.results.last() else {
            return;
        };
        if self.chart.is_none() {
            let columns = last.grid.read(cx).columns().to_vec();
            let (conn, sql) = (self.conn.clone(), last.sql.clone());
            let panel = cx.new(|cx| ChartPanel::new(conn, sql, columns, window, cx));
            let pin = cx.subscribe(&panel, |this, _, event: &PinChart, cx| this.pin(event, cx));
            let notify = cx.observe(&panel, |_, _, cx| cx.notify());
            self._subscriptions.extend([pin, notify]);
            self.chart = Some(panel);
        }
        self.active_tab = CHART_TAB;
        cx.notify();
    }

    /// Adds the chart to this connection's first dashboard, creating one if needed.
    fn pin(&mut self, event: &PinChart, cx: &mut Context<Self>) {
        let store = state::dashboards(cx);
        let mut dashboard = store
            .list(self.connection_id)
            .ok()
            .and_then(|d| d.into_iter().next())
            .unwrap_or_else(|| types::Dashboard {
                id: types::DashboardId::random(),
                connection: self.connection_id,
                title: "Overview".into(),
                cards: Vec::new(),
            });
        dashboard.cards.push(types::ChartCard {
            id: types::CardId::random(),
            title: event.title.clone(),
            subtitle: event.subtitle.clone(),
            chart: event.spec.clone(),
            refresh_secs: None,
            alert: None,
        });
        match store.save(&dashboard) {
            Ok(()) => cx.emit(Pinned),
            Err(e) => self.messages.push(Message {
                text: format!("Pin failed: {e}"),
                error: true,
            }),
        }
    }

    fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.history = match self.history.take() {
            Some(_) => None,
            None => Some(
                state::history(cx)
                    .recent(Some(self.connection_id), 50)
                    .unwrap_or_default(),
            ),
        };
        cx.notify();
    }

    fn active_data(&self, cx: &gpui::App) -> Option<(Vec<Column>, Vec<Vec<types::Value>>)> {
        let tab = self.results.get(self.active_tab)?;
        let grid = tab.grid.read(cx);
        Some((
            grid.columns().to_vec(),
            grid.values().map(<[types::Value]>::to_vec).collect(),
        ))
    }

    fn copy_as(&mut self, format: &str, cx: &mut Context<Self>) {
        if let Some((columns, rows)) = self.active_data(cx) {
            let rows = rows.iter().map(Vec::as_slice);
            let text = match format {
                "CSV" => export::csv(&columns, rows),
                "JSON" => export::json(&columns, rows),
                _ => export::inserts("result", &columns, rows),
            };
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        self.copy_menu = false;
        cx.notify();
    }

    fn export_csv(&mut self, cx: &mut Context<Self>) {
        let Some((columns, rows)) = self.active_data(cx) else {
            return;
        };
        let csv = export::csv(&columns, rows.iter().map(Vec::as_slice));
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let path = cx.prompt_for_new_path(&dir, Some("result.csv"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = path.await {
                let message = match std::fs::write(&path, csv) {
                    Ok(()) => (format!("Exported to {}", path.display()), false),
                    Err(e) => (format!("Export failed: {e}"), true),
                };
                let _ = this.update(cx, |view, cx| {
                    view.messages.push(Message {
                        text: message.0,
                        error: message.1,
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let cmd = if cfg!(target_os = "macos") {
            k.modifiers.platform
        } else {
            k.modifiers.control
        };
        match k.key.as_str() {
            "enter" if cmd && k.modifiers.shift => self.run_all(cx),
            "enter" if cmd => self.run_current(cx),
            "escape" if self.running.is_some() => self.cancel(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                ui::primary_icon_button("run-current", "play", "Run current")
                    .child(ui::kbd("⌘↵", cx).opacity(0.85))
                    .on_click(cx.listener(|this, _, _, cx| this.run_current(cx))),
            )
            .child(
                ui::secondary_button("run-all", "Run all")
                    .on_click(cx.listener(|this, _, _, cx| this.run_all(cx))),
            )
            .child(
                ui::ghost_button("explain", "Explain")
                    .on_click(cx.listener(|this, _, _, cx| this.explain(cx))),
            )
            .child(
                ui::ghost_button("format", "Format")
                    .on_click(cx.listener(|this, _, window, cx| this.format(window, cx))),
            )
            .when(self.running.is_some(), |d| {
                d.child(
                    ui::ghost_button("cancel", "Cancel (Esc)")
                        .text_color(c(theme::DELETED_TEXT))
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
            })
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(c(theme::FAINT))
                    .child(ui::tag_square(self.tag, 7.))
                    .child(self.label.clone())
                    .child(
                        div()
                            .relative()
                            .child(
                                ui::icon_button("history", "clock", 28., 15.).on_click(
                                    cx.listener(|this, _, _, cx| this.toggle_history(cx)),
                                ),
                            )
                            .children(self.history.as_ref().map(|h| self.render_history(h, cx))),
                    )
                    .child(
                        ui::icon_button("copy-query", "copy", 28., 15.).on_click(cx.listener(
                            |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.text(cx)));
                            },
                        )),
                    ),
            )
    }

    fn render_history(&self, entries: &[HistoryEntry], cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        deferred(
            anchored().snap_to_window().child(
                div()
                    .id("history-menu")
                    .occlude()
                    .mt(px(30.))
                    .w(px(420.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .rounded(px(6.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .shadow_md()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.history = None;
                        cx.notify();
                    }))
                    .when(entries.is_empty(), |d| {
                        d.child(
                            div()
                                .p(px(8.))
                                .text_color(c(theme::FAINT))
                                .child("No queries yet."),
                        )
                    })
                    .children(entries.iter().enumerate().map(|(i, entry)| {
                        let sql = entry.sql.clone();
                        let first = sql.lines().next().unwrap_or_default().to_owned();
                        let meta = format!(
                            "{} · {} ms{}",
                            entry.ran_at.format("%Y-%m-%d %H:%M"),
                            entry.duration_ms,
                            if entry.error.is_some() {
                                " · failed"
                            } else {
                                ""
                            }
                        );
                        div()
                            .id(("history-item", i))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|d| d.bg(c(theme::SELECTION)))
                            .child(
                                div()
                                    .font_family(mono.clone())
                                    .text_size(px(12.5))
                                    .text_color(c(theme::TEXT))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(SharedString::from(first)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(c(theme::FAINT))
                                    .child(SharedString::from(meta)),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let sql = sql.clone();
                                this.editor.update(cx, |s, cx| s.set_value(sql, window, cx));
                                this.history = None;
                                cx.notify();
                            }))
                    })),
            ),
        )
        .with_priority(1)
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |ix: usize, label: String, active: bool| {
            div()
                .id(("result-tab", ix))
                .flex()
                .items_center()
                .h(px(34.))
                .px(px(4.))
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .border_b_2()
                .border_color(if active {
                    c(theme::ACCENT)
                } else {
                    gpui::transparent_black().into()
                })
                .text_color(c(if active { theme::TEXT } else { theme::MUTED }))
                .child(SharedString::from(label))
        };
        let messages_ix = self.results.len();
        let mut tabs = div().flex().gap(px(18.));
        for (i, r) in self.results.iter().enumerate() {
            tabs = tabs.child(
                tab(
                    i,
                    format!("Result {} · {} rows", i + 1, ui::thousands(r.rows)),
                    self.active_tab == i,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active_tab = i;
                    cx.notify();
                })),
            );
        }
        if self.conn.aggregate().is_some() && !self.results.is_empty() {
            tabs = tabs.child(
                tab(CHART_TAB, "Chart".into(), self.active_tab == CHART_TAB)
                    .gap(px(6.))
                    .child(ui::icon("chart", 13., theme::MUTED))
                    .flex_row_reverse()
                    .on_click(cx.listener(|this, _, window, cx| this.open_chart(window, cx))),
            );
        }
        tabs = tabs.child(
            tab(
                messages_ix,
                "Messages".into(),
                self.active_tab == messages_ix,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.active_tab = messages_ix;
                cx.notify();
            })),
        );
        let has_rows = self.active_tab < self.results.len();
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(18.))
            .px(px(12.))
            .bg(c(theme::PANEL))
            .border_b_1()
            .border_color(c(theme::LINE))
            .child(tabs)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        ui::ghost_button("export", "Export")
                            .flex_row_reverse()
                            .child(ui::icon("download", 14., theme::MUTED))
                            .when(has_rows, |b| {
                                b.on_click(cx.listener(|this, _, _, cx| this.export_csv(cx)))
                            }),
                    )
                    .child(
                        div()
                            .relative()
                            .child(
                                ui::ghost_button("copy-as", "Copy as")
                                    .flex_row_reverse()
                                    .child(ui::icon("copy", 14., theme::MUTED))
                                    .when(has_rows, |b| {
                                        b.on_click(cx.listener(|this, _, _, cx| {
                                            this.copy_menu = !this.copy_menu;
                                            cx.notify();
                                        }))
                                    }),
                            )
                            .children(self.copy_menu.then(|| {
                                deferred(
                                    anchored().snap_to_window().child(
                                        div()
                                            .id("copy-menu")
                                            .occlude()
                                            .mt(px(30.))
                                            .w(px(140.))
                                            .p(px(4.))
                                            .rounded(px(6.))
                                            .bg(c(theme::BG))
                                            .border_1()
                                            .border_color(c(theme::LINE))
                                            .shadow_md()
                                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                                this.copy_menu = false;
                                                cx.notify();
                                            }))
                                            .children(["CSV", "JSON", "INSERT"].into_iter().map(
                                                |f| {
                                                    div()
                                                        .id(SharedString::from(format!("copy-{f}")))
                                                        .h(px(28.))
                                                        .px(px(8.))
                                                        .flex()
                                                        .items_center()
                                                        .rounded(px(4.))
                                                        .text_size(px(13.))
                                                        .cursor_pointer()
                                                        .hover(|d| d.bg(c(theme::SELECTION)))
                                                        .child(f)
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.copy_as(f, cx)
                                                            },
                                                        ))
                                                },
                                            )),
                                    ),
                                )
                                .with_priority(1)
                            })),
                    ),
            );
        let chart = self.chart.clone().filter(|_| self.active_tab == CHART_TAB);
        let body = match (self.results.get(self.active_tab), chart) {
            (_, Some(chart)) => chart.into_any_element(),
            (Some(r), _) => r.grid.clone().into_any_element(),
            (None, None) => {
                let mono = theme::mono(cx);
                div()
                    .id("messages")
                    .size_full()
                    .overflow_y_scroll()
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .font_family(mono)
                    .text_size(px(12.5))
                    .children(self.messages.iter().map(|m| {
                        div()
                            .text_color(c(if m.error {
                                theme::DELETED_TEXT
                            } else {
                                theme::TEXT
                            }))
                            .child(SharedString::from(m.text.clone()))
                    }))
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .border_t_1()
            .border_color(c(theme::LINE))
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let position = self.editor.read(cx).cursor_position();
        let sep = || div().text_color(c(theme::FAINT)).child("·");
        let chart_status = self
            .chart
            .as_ref()
            .filter(|_| self.active_tab == CHART_TAB)
            .and_then(|c| c.read(cx).status());
        let left = match (&self.summary, self.running.is_some()) {
            (_, true) => div().child("Running…  Esc to cancel"),
            _ if let Some((ok, text)) = chart_status => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(c(if ok {
                    theme::MUTED
                } else {
                    theme::DELETED_TEXT
                }))
                .children(ok.then(|| ui::icon("check", 13., theme::NEW_TEXT)))
                .children(ok.then(|| div().text_color(c(theme::NEW_TEXT)).child("Query OK")))
                .child(SharedString::from(text)),
            (Some(s), false) if s.ok => div()
                .flex()
                .items_center()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(c(theme::NEW_TEXT))
                        .child(ui::icon("check", 13., theme::NEW_TEXT))
                        .child("Query OK"),
                )
                .child(SharedString::from(format!(
                    "{} rows",
                    ui::thousands(s.rows)
                )))
                .child(sep())
                .child(SharedString::from(format!("{} ms", s.ms)))
                .child(sep())
                .child(SharedString::from(s.text.clone())),
            (Some(s), false) => div()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(c(theme::DELETED_TEXT))
                .child(SharedString::from(s.text.clone())),
            (None, false) => div()
                .text_color(c(theme::FAINT))
                .child("⌘↵ runs the statement under the cursor"),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(14.))
            .px(px(12.))
            .h(px(37.))
            .border_t_1()
            .border_color(c(theme::LINE))
            .bg(c(theme::PANEL))
            .text_size(px(12.))
            .text_color(c(theme::MUTED))
            .child(div().flex_1().min_w_0().child(left))
            .child(
                div()
                    .text_color(c(theme::FAINT))
                    .child(SharedString::from(format!(
                        "Ln {}, Col {}",
                        position.line + 1,
                        position.character + 1
                    ))),
            )
    }
}

impl Drop for SqlEditor {
    fn drop(&mut self) {
        if let Some(token) = &self.running {
            token.cancel();
        }
    }
}

impl Render for SqlEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        div()
            .id("sql-editor")
            .key_context("SqlEditor")
            .capture_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex_none()
                    .h(px(EDITOR_HEIGHT))
                    .pb(px(12.))
                    .bg(c(theme::BG))
                    .font_family(mono)
                    .text_size(px(13.))
                    .line_height(px(21.))
                    .child(Editor::new(&self.editor).appearance(false).h_full()),
            )
            .child(self.render_results(cx))
            .child(self.render_status(cx))
    }
}
