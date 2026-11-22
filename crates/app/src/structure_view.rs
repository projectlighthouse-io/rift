//! Table structure: columns, indexes and foreign keys, with pending changes shown as SQL and
//! applied with ⌘S. Shown only when the connection has `describe()`; editing controls need
//! `edit_schema()`.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, SharedString, Task, Window, div, px,
};
use gpui_component::input::InputState;
use traits::Connection;
use types::{ColumnDef, Error, IndexDef, SchemaChange, TableDescription, TableRef};

use crate::theme::{self, c};
use crate::{runtime, ui};

/// What the structure view asks its table view to do.
pub(crate) enum StructureEvent {
    /// Switch back to the Data view.
    ShowData,
    /// The table's structure or rows changed; reload data.
    Changed,
}

struct ColumnForm {
    name: Entity<InputState>,
    type_name: Entity<InputState>,
    default: Entity<InputState>,
    comment: Entity<InputState>,
    nullable: bool,
}

struct IndexForm {
    name: Entity<InputState>,
    columns: Entity<InputState>,
    unique: bool,
}

/// Structure tab of one table.
pub(crate) struct StructureView {
    conn: Arc<dyn Connection>,
    table: TableRef,
    desc: Option<TableDescription>,
    error: Option<String>,
    pending: Vec<SchemaChange>,
    ddl: Option<String>,
    confirm_truncate: bool,
    column_form: Option<ColumnForm>,
    index_form: Option<IndexForm>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<StructureEvent> for StructureView {}

/// Column widths of the three tables; the last column takes the rest.
const COLUMN_WIDTHS: [f32; 5] = [214., 167., 100., 281., 78.];
const INDEX_WIDTHS: [f32; 4] = [404., 375., 130., 125.];
const FK_WIDTHS: [f32; 4] = [379., 205., 234., 162.];

impl StructureView {
    pub(crate) fn new(conn: Arc<dyn Connection>, table: TableRef, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            conn,
            table,
            desc: None,
            error: None,
            pending: Vec::new(),
            ddl: None,
            confirm_truncate: false,
            column_form: None,
            index_form: None,
            _tasks: Vec::new(),
        };
        view.reload(cx);
        view
    }

    fn editable(&self) -> bool {
        self.conn.edit_schema().is_some()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let table = self.table.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.describe() {
                    Some(d) => d.describe(&table).await,
                    None => Err(Error::unsupported("describe")),
                }
            },
            |this, result, cx| {
                match result {
                    Ok(desc) => {
                        this.desc = Some(desc);
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    fn toggle_ddl(&mut self, cx: &mut Context<Self>) {
        if self.ddl.take().is_some() {
            cx.notify();
            return;
        }
        let conn = self.conn.clone();
        let table = self.table.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.describe() {
                    Some(d) => d.ddl(&table).await,
                    None => Err(Error::unsupported("describe")),
                }
            },
            |this, result, cx| {
                match result {
                    Ok(ddl) => this.ddl = Some(ddl),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    fn preview(&self) -> Result<Vec<String>, Error> {
        match self.conn.edit_schema() {
            Some(e) => e.preview(&self.table, &self.pending),
            None => Err(Error::unsupported("edit schema")),
        }
    }

    /// Runs changes now. Used for pending changes (⌘S) and for Truncate.
    fn run(&mut self, changes: Vec<SchemaChange>, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let table = self.table.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.edit_schema() {
                    Some(e) => e.apply_schema(&table, &changes).await,
                    None => Err(Error::unsupported("edit schema")),
                }
            },
            |this, result, cx| {
                match result {
                    Ok(()) => {
                        this.pending.clear();
                        this.error = None;
                        this.ddl = None;
                        cx.emit(StructureEvent::Changed);
                        this.reload(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    /// Applies pending changes (⌘S).
    pub(crate) fn apply(&mut self, cx: &mut Context<Self>) {
        if !self.pending.is_empty() {
            let changes = self.pending.clone();
            self.run(changes, cx);
        }
    }

    fn truncate(&mut self, cx: &mut Context<Self>) {
        if self.confirm_truncate {
            self.confirm_truncate = false;
            self.run(vec![SchemaChange::Truncate], cx);
        } else {
            self.confirm_truncate = true;
            cx.notify();
        }
    }

    fn toggle_drop_column(&mut self, name: &types::ColumnName, cx: &mut Context<Self>) {
        let existing = self
            .pending
            .iter()
            .position(|c| matches!(c, SchemaChange::DropColumn { name: n } if n == name));
        match existing {
            Some(i) => {
                self.pending.remove(i);
            }
            None => self
                .pending
                .push(SchemaChange::DropColumn { name: name.clone() }),
        }
        cx.notify();
    }

    fn toggle_drop_index(&mut self, name: &types::IndexName, cx: &mut Context<Self>) {
        let existing = self
            .pending
            .iter()
            .position(|c| matches!(c, SchemaChange::DropIndex { name: n } if n == name));
        match existing {
            Some(i) => {
                self.pending.remove(i);
            }
            None => self
                .pending
                .push(SchemaChange::DropIndex { name: name.clone() }),
        }
        cx.notify();
    }

    fn open_column_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_owned();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let form = ColumnForm {
            name: input("column_name", window, cx),
            type_name: input("text", window, cx),
            default: input("—", window, cx),
            comment: input("Comment", window, cx),
            nullable: true,
        };
        form.name.update(cx, |s, cx| s.focus(window, cx));
        self.column_form = Some(form);
        cx.notify();
    }

    fn submit_column_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.column_form else {
            return;
        };
        let text = |e: &Entity<InputState>| e.read(cx).value().trim().to_owned();
        let (name, type_name) = (text(&form.name), text(&form.type_name));
        if name.is_empty() || type_name.is_empty() {
            self.error = Some("A new column needs a name and a type.".into());
            cx.notify();
            return;
        }
        let default = text(&form.default);
        let comment = text(&form.comment);
        self.pending.push(SchemaChange::AddColumn {
            column: ColumnDef {
                name: name.into(),
                type_name,
                nullable: form.nullable,
                default: (!default.is_empty()).then_some(default),
                primary_key: false,
                references: None,
                comment: (!comment.is_empty()).then_some(comment),
            },
        });
        self.column_form = None;
        self.error = None;
        cx.notify();
    }

    fn open_index_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let table = self.table.name.to_string();
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(format!("{table}_col_idx")));
        let columns =
            cx.new(|cx| InputState::new(window, cx).placeholder("column, other_column DESC"));
        name.update(cx, |s, cx| s.focus(window, cx));
        self.index_form = Some(IndexForm {
            name,
            columns,
            unique: false,
        });
        cx.notify();
    }

    fn submit_index_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.index_form else { return };
        let name = form.name.read(cx).value().trim().to_owned();
        let columns: Vec<String> = form
            .columns
            .read(cx)
            .value()
            .split(',')
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty())
            .collect();
        if name.is_empty() || columns.is_empty() {
            self.error = Some("A new index needs a name and columns.".into());
            cx.notify();
            return;
        }
        self.pending.push(SchemaChange::AddIndex {
            index: IndexDef {
                name: name.into(),
                columns,
                method: None,
                unique: form.unique,
                primary: false,
                size_bytes: None,
            },
        });
        self.index_form = None;
        self.error = None;
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = self.desc.as_ref().map(|d| {
            let mut parts = vec![
                format!("{} columns", d.columns.len()),
                format!("{} indexes", d.indexes.len()),
            ];
            if let Some(rows) = d.row_estimate {
                parts.push(format!("~{} rows", ui::thousands(rows)));
            }
            if let Some(bytes) = d.size_bytes {
                parts.push(ui::bytes(bytes));
            }
            parts.join(" · ")
        });
        div()
            .flex()
            .flex_wrap()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(c(theme::LINE))
            .child(ui::segmented(
                [("Data", false), ("Structure", true)],
                cx.listener(|_, ix: &usize, _, cx| {
                    if *ix == 0 {
                        cx.emit(StructureEvent::ShowData);
                    }
                }),
            ))
            .children(summary.map(|s| {
                div()
                    .ml(px(8.))
                    .text_size(px(12.))
                    .text_color(c(theme::FAINT))
                    .child(SharedString::from(s))
            }))
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .gap(px(6.))
                    .child(
                        ui::ghost_button(
                            "show-ddl",
                            if self.ddl.is_some() {
                                "Hide DDL"
                            } else {
                                "Show DDL"
                            },
                        )
                        .flex_row_reverse()
                        .child(ui::icon("code", 14., theme::MUTED))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_ddl(cx))),
                    )
                    .when(self.editable(), |d| {
                        d.child(
                            ui::ghost_button(
                                "truncate",
                                if self.confirm_truncate {
                                    "Delete all rows?"
                                } else {
                                    "Truncate…"
                                },
                            )
                            .text_color(c(theme::DELETED_TEXT))
                            .border_1()
                            .border_color(gpui::rgb(0xf1bcb7))
                            .when(self.confirm_truncate, |b| b.bg(c(theme::DELETED_BG)))
                            .on_click(cx.listener(|this, _, _, cx| this.truncate(cx))),
                        )
                    }),
            )
    }

    fn section(
        &self,
        title: &'static str,
        count: Option<usize>,
        add: Option<(&'static str, AnyElement)>,
        table: AnyElement,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .children(count.map(|n| {
                        div()
                            .text_size(px(12.))
                            .text_color(c(theme::FAINT))
                            .child(SharedString::from(n.to_string()))
                    }))
                    .children(add.map(|(_, button)| div().ml_auto().child(button))),
            )
            .child(table)
    }

    fn render_columns(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mono = theme::mono(cx);
        let Some(desc) = &self.desc else {
            return div().into_any_element();
        };
        let headers = ["Name", "Type", "Nullable", "Default", "Key", "Comment"];
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, col) in desc.columns.iter().enumerate() {
            let dropped = self
                .pending
                .iter()
                .any(|c| matches!(c, SchemaChange::DropColumn { name } if *name == col.name));
            let key = if col.primary_key {
                key_badge("key", "PK", theme::EDITED_TEXT)
            } else if col.references.is_some() {
                key_badge("link", "FK", theme::ACCENT_TEXT)
            } else {
                dash()
            };
            let comment: SharedString = match (&col.references, &col.comment) {
                (_, Some(text)) => text.clone().into(),
                (Some(r), None) => format!("→ {}.{}", r.table.name, r.column).into(),
                (None, None) => SharedString::default(),
            };
            let name = col.name.clone();
            let cells = vec![
                mono_text(col.name.to_string(), theme::TEXT, &mono),
                mono_text(col.type_name.clone(), theme::ACCENT_TEXT, &mono),
                if col.nullable {
                    ui::icon("check", 14., theme::NEW_TEXT).into_any_element()
                } else {
                    dash()
                },
                match &col.default {
                    Some(d) => mono_text(crate::table_view::clean_default(d), theme::MUTED, &mono),
                    None => dash(),
                },
                key,
                div().child(comment).into_any_element(),
            ];
            let trailing = self.editable().then(|| {
                ui::icon_button(("drop-col", i), "trash", 24., 13.)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_drop_column(&name, cx)))
                    .into_any_element()
            });
            rows.push(table_row(
                i,
                &COLUMN_WIDTHS,
                cells,
                trailing,
                row_state(dropped, false),
            ));
        }
        let mut next = desc.columns.len();
        for (p, change) in self.pending.iter().enumerate() {
            if let SchemaChange::AddColumn { column } = change {
                let cells = vec![
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(mono_text(column.name.to_string(), theme::NEW_TEXT, &mono))
                        .child(ui::badge("NEW", theme::NEW_TEXT, theme::NEW_BG))
                        .into_any_element(),
                    mono_text(column.type_name.clone(), theme::ACCENT_TEXT, &mono),
                    if column.nullable {
                        ui::icon("check", 14., theme::NEW_TEXT).into_any_element()
                    } else {
                        dash()
                    },
                    column
                        .default
                        .clone()
                        .map_or_else(dash, |d| mono_text(d, theme::MUTED, &mono)),
                    dash(),
                    div()
                        .child(SharedString::from(
                            column.comment.clone().unwrap_or_default(),
                        ))
                        .into_any_element(),
                ];
                let remove = ui::icon_button(("undo-add", p), "close", 24., 13.)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pending.remove(p);
                        cx.notify();
                    }))
                    .into_any_element();
                rows.push(table_row(
                    next,
                    &COLUMN_WIDTHS,
                    cells,
                    Some(remove),
                    row_state(false, true),
                ));
                next += 1;
            }
        }
        if let Some(form) = &self.column_form {
            let nullable = form.nullable;
            let cells = vec![
                field(&form.name, &mono),
                field(&form.type_name, &mono),
                div()
                    .id("new-col-nullable")
                    .size(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .cursor_pointer()
                    .children(nullable.then(|| ui::icon("check", 13., theme::NEW_TEXT)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(f) = &mut this.column_form {
                            f.nullable = !f.nullable;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                field(&form.default, &mono),
                dash(),
                field(&form.comment, &mono),
            ];
            let add = ui::secondary_button("submit-col", "Add")
                .h(px(24.))
                .on_click(cx.listener(|this, _, _, cx| this.submit_column_form(cx)))
                .into_any_element();
            rows.push(table_row(
                next,
                &COLUMN_WIDTHS,
                cells,
                Some(add),
                RowState::Plain,
            ));
        }
        let _ = window;
        bordered_table(&headers, &COLUMN_WIDTHS, rows)
    }

    fn render_indexes(&self, cx: &mut Context<Self>) -> AnyElement {
        let mono = theme::mono(cx);
        let Some(desc) = &self.desc else {
            return div().into_any_element();
        };
        let headers = ["Name", "Columns", "Method", "Unique", "Size"];
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, index) in desc.indexes.iter().enumerate() {
            let dropped = self
                .pending
                .iter()
                .any(|c| matches!(c, SchemaChange::DropIndex { name } if *name == index.name));
            let name = index.name.clone();
            let cells = vec![
                mono_text(index.name.to_string(), theme::TEXT, &mono),
                mono_text(index.columns.join(", "), theme::TEXT, &mono),
                div()
                    .child(SharedString::from(index.method.clone().unwrap_or_default()))
                    .into_any_element(),
                if index.unique {
                    ui::icon("check", 14., theme::NEW_TEXT).into_any_element()
                } else {
                    dash()
                },
                div()
                    .child(SharedString::from(
                        index.size_bytes.map(ui::bytes).unwrap_or_default(),
                    ))
                    .into_any_element(),
            ];
            let trailing = (self.editable() && !index.primary).then(|| {
                ui::icon_button(("drop-idx", i), "trash", 24., 13.)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_drop_index(&name, cx)))
                    .into_any_element()
            });
            rows.push(table_row(
                i,
                &INDEX_WIDTHS,
                cells,
                trailing,
                row_state(dropped, false),
            ));
        }
        let mut next = desc.indexes.len();
        for (p, change) in self.pending.iter().enumerate() {
            if let SchemaChange::AddIndex { index } = change {
                let cells = vec![
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(mono_text(index.name.to_string(), theme::NEW_TEXT, &mono))
                        .child(ui::badge("NEW", theme::NEW_TEXT, theme::NEW_BG))
                        .into_any_element(),
                    mono_text(index.columns.join(", "), theme::TEXT, &mono),
                    div().child("btree").into_any_element(),
                    if index.unique {
                        ui::icon("check", 14., theme::NEW_TEXT).into_any_element()
                    } else {
                        dash()
                    },
                    dash(),
                ];
                let remove = ui::icon_button(("undo-idx", p), "close", 24., 13.)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pending.remove(p);
                        cx.notify();
                    }))
                    .into_any_element();
                rows.push(table_row(
                    next,
                    &INDEX_WIDTHS,
                    cells,
                    Some(remove),
                    row_state(false, true),
                ));
                next += 1;
            }
        }
        if let Some(form) = &self.index_form {
            let unique = form.unique;
            let cells = vec![
                field(&form.name, &mono),
                field(&form.columns, &mono),
                div().child("btree").into_any_element(),
                div()
                    .id("new-idx-unique")
                    .size(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .cursor_pointer()
                    .children(unique.then(|| ui::icon("check", 13., theme::NEW_TEXT)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(f) = &mut this.index_form {
                            f.unique = !f.unique;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                dash(),
            ];
            let add = ui::secondary_button("submit-idx", "Add")
                .h(px(24.))
                .on_click(cx.listener(|this, _, _, cx| this.submit_index_form(cx)))
                .into_any_element();
            rows.push(table_row(
                next,
                &INDEX_WIDTHS,
                cells,
                Some(add),
                RowState::Plain,
            ));
        }
        bordered_table(&headers, &INDEX_WIDTHS, rows)
    }

    fn render_foreign_keys(&self, cx: &mut Context<Self>) -> AnyElement {
        let mono = theme::mono(cx);
        let Some(desc) = &self.desc else {
            return div().into_any_element();
        };
        let headers = ["Name", "Column", "References", "On delete", "On update"];
        let rows = desc
            .foreign_keys
            .iter()
            .enumerate()
            .map(|(i, fk)| {
                let column = fk
                    .columns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let target = format!(
                    "{}({})",
                    fk.ref_table.name,
                    fk.ref_columns
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                let cells = vec![
                    mono_text(fk.name.clone(), theme::TEXT, &mono),
                    mono_text(column, theme::TEXT, &mono),
                    mono_text(target, theme::TEXT, &mono),
                    div()
                        .child(SharedString::from(fk.on_delete.clone()))
                        .into_any_element(),
                    div()
                        .child(SharedString::from(fk.on_update.clone()))
                        .into_any_element(),
                ];
                table_row(i, &FK_WIDTHS, cells, None, RowState::Plain)
            })
            .collect();
        bordered_table(&headers, &FK_WIDTHS, rows)
    }

    fn render_status(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.pending.is_empty() && self.error.is_none() {
            return None;
        }
        let mono = theme::mono(cx);
        let (text, color) = match (&self.error, self.preview()) {
            (Some(e), _) => (e.clone(), theme::DELETED_TEXT),
            (None, Ok(sql)) => (format!("{};", sql.join("; ")), theme::MUTED),
            (None, Err(e)) => (e.to_string(), theme::DELETED_TEXT),
        };
        let n = self.pending.len();
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(14.))
                .px(px(12.))
                .h(px(45.))
                .border_t_1()
                .border_color(c(theme::LINE))
                .bg(c(theme::PANEL))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(mono)
                        .text_size(px(12.5))
                        .text_color(c(color))
                        .child(SharedString::from(text)),
                )
                .when(
                    n > 0
                        && !self
                            .conn
                            .edit_schema()
                            .is_some_and(|e| e.transactional_ddl()),
                    |d| {
                        d.child(ui::badge(
                            "Cannot be rolled back",
                            theme::EDITED_TEXT,
                            theme::EDITED_BG,
                        ))
                    },
                )
                .when(n > 0, |d| {
                    d.child(ui::badge(
                        format!("{n} pending change{}", if n == 1 { "" } else { "s" }),
                        theme::NEW_TEXT,
                        theme::NEW_BG,
                    ))
                    .child(
                        ui::ghost_button("discard-schema", "Discard").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.pending.clear();
                                this.error = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        ui::primary_button("apply-schema", "Apply")
                            .child(ui::kbd("⌘S", cx).opacity(0.85))
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    )
                }),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowState {
    Plain,
    New,
    Dropped,
}

fn row_state(dropped: bool, new: bool) -> RowState {
    if dropped {
        RowState::Dropped
    } else if new {
        RowState::New
    } else {
        RowState::Plain
    }
}

fn dash() -> AnyElement {
    div()
        .text_color(c(theme::FAINT))
        .child("—")
        .into_any_element()
}

fn mono_text(text: String, color: u32, mono: &SharedString) -> AnyElement {
    div()
        .font_family(mono.clone())
        .text_color(c(color))
        .overflow_hidden()
        .text_ellipsis()
        .child(SharedString::from(text))
        .into_any_element()
}

fn key_badge(icon: &str, label: &'static str, color: u32) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_color(c(color))
        .child(ui::icon(icon, 12., color))
        .child(label)
        .into_any_element()
}

fn field(input: &Entity<InputState>, mono: &SharedString) -> AnyElement {
    div()
        .w_full()
        .h(px(24.))
        .rounded(px(4.))
        .bg(c(theme::BG))
        .border_1()
        .border_color(c(theme::LINE))
        .font_family(mono.clone())
        .child(ui::input(input, 12.5))
        .into_any_element()
}

/// A 32px row: fixed-width cells, the last cell flexes, optional trailing action.
fn table_row(
    ix: usize,
    widths: &[f32],
    cells: Vec<AnyElement>,
    trailing: Option<AnyElement>,
    state: RowState,
) -> AnyElement {
    let bg = match state {
        RowState::New => theme::NEW_BG,
        RowState::Dropped => theme::DELETED_BG,
        RowState::Plain if ix % 2 == 1 => theme::PANEL,
        RowState::Plain => theme::BG,
    };
    let count = cells.len();
    div()
        .flex()
        .items_center()
        .h(px(32.))
        .bg(c(bg))
        .border_t_1()
        .border_color(c(theme::LINE))
        .text_size(px(12.5))
        .when(state == RowState::Dropped, |d| {
            d.line_through().text_color(c(theme::DELETED_TEXT))
        })
        .children(cells.into_iter().enumerate().map(|(i, cell)| {
            let base = div()
                .px(px(12.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis();
            if i + 1 == count {
                base.flex_1().min_w_0()
            } else {
                base.w(px(widths[i])).flex_none()
            }
            .child(cell)
        }))
        .child(
            div()
                .w(px(36.))
                .flex_none()
                .flex()
                .justify_center()
                .children(trailing),
        )
        .into_any_element()
}

fn bordered_table(headers: &[&str], widths: &[f32], rows: Vec<AnyElement>) -> AnyElement {
    let count = headers.len();
    div()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(c(theme::LINE))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .h(px(32.))
                .bg(c(theme::PANEL))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(theme::MUTED))
                .children(headers.iter().enumerate().map(|(i, h)| {
                    let base = div().px(px(12.));
                    if i + 1 == count {
                        base.flex_1()
                    } else {
                        base.w(px(widths[i])).flex_none()
                    }
                    .child(SharedString::from(*h))
                }))
                .child(div().w(px(36.)).flex_none()),
        )
        .children(rows)
        .into_any_element()
}

impl Render for StructureView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("StructureView::render").entered();
        let mono = theme::mono(cx);
        let editable = self.editable();
        let add_column = editable.then(|| {
            (
                "Add column",
                ui::ghost_button("add-column", "Add column")
                    .flex_row_reverse()
                    .child(ui::icon("plus", 14., theme::MUTED))
                    .on_click(cx.listener(|this, _, window, cx| this.open_column_form(window, cx)))
                    .into_any_element(),
            )
        });
        let add_index = editable.then(|| {
            (
                "Add index",
                ui::ghost_button("add-index", "Add index")
                    .flex_row_reverse()
                    .child(ui::icon("plus", 14., theme::MUTED))
                    .on_click(cx.listener(|this, _, window, cx| this.open_index_form(window, cx)))
                    .into_any_element(),
            )
        });
        let ddl = self.ddl.clone().map(|ddl| {
            div()
                .flex()
                .flex_col()
                .p(px(12.))
                .rounded(px(8.))
                .bg(c(theme::BG))
                .border_1()
                .border_color(c(theme::LINE))
                .font_family(mono.clone())
                .text_size(px(12.5))
                .children(
                    ddl.lines()
                        .map(|l| {
                            div()
                                .whitespace_nowrap()
                                .child(SharedString::from(l.to_owned()))
                        })
                        .collect::<Vec<_>>(),
                )
        });
        let (index_count, fk_count) = self.desc.as_ref().map_or((None, None), |d| {
            (Some(d.indexes.len()), Some(d.foreign_keys.len()))
        });
        let columns = self.render_columns(window, cx);
        let indexes = self.render_indexes(cx);
        let fks = self.render_foreign_keys(cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("structure-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .py(px(20.))
                    .gap(px(28.))
                    .children(ddl)
                    .child(self.section("Columns", None, add_column, columns))
                    .child(self.section("Indexes", index_count, add_index, indexes))
                    .child(self.section("Foreign keys", fk_count, None, fks)),
            )
            .children(self.render_status(cx))
    }
}
