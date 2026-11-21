//! A table document: filters, paged grid, inline edits, row inspector, and saving edits in one
//! transaction (`EditRows`). No review dialog: ⌘S saves.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, SharedString, Subscription,
    Task, Window, anchored, deferred, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use traits::Connection;
use types::{
    Column, ColumnValue, Error, Filter, FilterOp, Row, RowEdit, RowEditKind, RowsRequest, SafeMode,
    TableRef, Value,
};

use crate::actions::SaveChanges;
use crate::column_stats::{ColumnStatsView, StatsAction};
use crate::dropdown::Dropdown;
use crate::grid::{Grid, GridEvent, GridRow, RowMark};
use crate::structure_view::{StructureEvent, StructureView};
use crate::theme::{self, c};
use crate::{runtime, ui};

/// Rows per page in the table view.
const PAGE: u64 = 50;
const INSPECTOR_WIDTH: f32 = 260.0;

/// Filter operators in picker order.
const OPS: [(&str, FilterOp); 9] = [
    ("is", FilterOp::Eq),
    ("is not", FilterOp::NotEq),
    ("<", FilterOp::Lt),
    ("≤", FilterOp::Lte),
    (">", FilterOp::Gt),
    ("≥", FilterOp::Gte),
    ("contains", FilterOp::Contains),
    ("is null", FilterOp::IsNull),
    ("is not null", FilterOp::IsNotNull),
];

/// Asks the session to open another table, filtered (FK links).
/// Asks the session to open a query tab with this SQL.
pub(crate) struct OpenQuery(pub(crate) String);

pub(crate) struct OpenTable {
    pub(crate) table: TableRef,
    pub(crate) filter: Filter,
}

struct FilterRow {
    column: Entity<Dropdown>,
    op: Entity<Dropdown>,
    value: Entity<InputState>,
}

struct Inspector {
    row: usize,
    inputs: Vec<Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
}

/// Pending, unsaved edits. Indexes are page rows; inserts are appended after them.
#[derive(Default)]
struct Pending {
    updates: BTreeMap<usize, BTreeMap<usize, Value>>,
    inserts: Vec<BTreeMap<usize, Value>>,
    deletes: BTreeSet<usize>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.updates.is_empty() && self.inserts.is_empty() && self.deletes.is_empty()
    }

    fn edited_cells(&self) -> usize {
        self.updates
            .iter()
            .filter(|(r, _)| !self.deletes.contains(r))
            .map(|(_, m)| m.len())
            .sum()
    }
}

/// Data, paging and edits for one table.
pub(crate) struct TableView {
    conn: Arc<dyn Connection>,
    pub(crate) table: TableRef,
    grid: Entity<Grid>,
    focus: FocusHandle,
    columns: Vec<Column>,
    rows: Vec<Row>,
    /// Column defaults from `Describe`, shown in new rows.
    defaults: Vec<Option<String>>,
    hidden: BTreeSet<usize>,
    pending: Pending,
    page: u64,
    total: Option<u64>,
    elapsed: Option<Duration>,
    error: Option<String>,
    filters: Vec<FilterRow>,
    applied: Vec<Filter>,
    selected: Option<usize>,
    inspector: Option<Inspector>,
    columns_menu: bool,
    structure: Option<Entity<StructureView>>,
    show_structure: bool,
    stats: Option<Entity<ColumnStatsView>>,
    safe_mode: SafeMode,
    confirm_save: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<OpenTable> for TableView {}
impl EventEmitter<OpenQuery> for TableView {}

impl TableView {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        table: TableRef,
        initial: Option<Filter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let grid = cx.new(|_| Grid::new());
        let subscription = cx.subscribe_in(
            &grid,
            window,
            |this, _, event: &GridEvent, window, cx| match event {
                GridEvent::Selected { row } => this.select(*row, window, cx),
                GridEvent::Header { col } => this.toggle_stats(*col, window, cx),
                GridEvent::Edited { row, col, text } => {
                    let col = this.visible_columns()[*col];
                    this.set_cell(*row, col, parse_cell(text), cx);
                    this.select(*row, window, cx);
                    // The editor is gone; keep ⌘S working.
                    window.focus(&this.focus, cx);
                }
            },
        );
        let mut view = Self {
            conn,
            table,
            grid,
            focus: cx.focus_handle(),
            columns: Vec::new(),
            rows: Vec::new(),
            defaults: Vec::new(),
            hidden: BTreeSet::new(),
            pending: Pending::default(),
            page: 0,
            total: None,
            elapsed: None,
            error: None,
            filters: Vec::new(),
            applied: initial.clone().into_iter().collect(),
            selected: None,
            inspector: None,
            columns_menu: false,
            structure: None,
            show_structure: false,
            stats: None,
            safe_mode: SafeMode::Off,
            confirm_save: false,
            _tasks: Vec::new(),
            _subscriptions: vec![subscription],
        };
        view.add_filter(initial, window, cx);
        view.reload(cx);
        view
    }

    fn has_key(&self) -> bool {
        self.columns.iter().any(|c| c.primary_key)
    }

    fn editable(&self) -> bool {
        self.has_key() && self.conn.edit_rows().is_some()
    }

    fn visible_columns(&self) -> Vec<usize> {
        (0..self.columns.len())
            .filter(|i| !self.hidden.contains(i))
            .collect()
    }

    fn load_defaults(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let table = self.table.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.describe() {
                    Some(d) => d.describe(&table).await.ok(),
                    None => None,
                }
            },
            |this, desc, cx| {
                if let Some(desc) = desc {
                    this.defaults = this
                        .columns
                        .iter()
                        .map(|col| {
                            desc.columns
                                .iter()
                                .find(|d| d.name == col.name)
                                .and_then(|d| d.default.as_deref())
                                .map(clean_default)
                        })
                        .collect();
                    this.refresh_grid(cx);
                }
            },
        ));
    }

    fn request(&self) -> RowsRequest {
        RowsRequest {
            filters: self.applied.clone(),
            offset: self.page * PAGE,
            limit: PAGE,
            ..RowsRequest::default()
        }
    }

    /// Reloads the page, dropping pending edits.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let table = self.table.clone();
        let request = self.request();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                let browse = conn.browse().ok_or_else(|| Error::unsupported("browse"))?;
                let started = Instant::now();
                let page = browse.rows(&table, &request).await?;
                let elapsed = started.elapsed();
                let total = browse.count(&table, &request).await?;
                Ok::<_, Error>((page, elapsed, total))
            },
            move |this, result, cx| {
                match result {
                    Ok((page, elapsed, total)) => {
                        let reshaped = page.columns != this.columns;
                        this.elapsed = Some(elapsed);
                        this.total = Some(total);
                        this.error = None;
                        this.columns = page.columns;
                        this.rows = page.rows;
                        this.pending = Pending::default();
                        this.selected = None;
                        this.inspector = None;
                        if reshaped {
                            this.sync_filter_columns(cx);
                            this.load_defaults(cx);
                        }
                        this.refresh_grid(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    /// The value shown for a cell, with pending edits applied.
    fn cell(&self, row: usize, col: usize) -> Value {
        if row < self.rows.len() {
            return self
                .pending
                .updates
                .get(&row)
                .and_then(|m| m.get(&col))
                .cloned()
                .unwrap_or_else(|| self.rows[row].values[col].clone());
        }
        let insert = &self.pending.inserts[row - self.rows.len()];
        insert.get(&col).cloned().unwrap_or_else(|| {
            match self.defaults.get(col).cloned().flatten() {
                // Generated keys are left blank, like the design.
                Some(d) if d.starts_with("nextval(") => Value::Text(String::new()),
                // `'pending'` shows as `pending`; expressions like `now()` stay as written.
                Some(d) => Value::Unknown {
                    type_name: String::new(),
                    raw: d
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                        .map_or(d.clone(), |s| s.replace("''", "'")),
                },
                None => Value::Null,
            }
        })
    }

    fn display_rows(&self) -> usize {
        self.rows.len() + self.pending.inserts.len()
    }

    fn refresh_grid(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible_columns();
        let columns: Vec<Column> = visible.iter().map(|&i| self.columns[i].clone()).collect();
        let rows: Vec<GridRow> = (0..self.display_rows())
            .map(|r| {
                let mark = if r >= self.rows.len() {
                    RowMark::New
                } else if self.pending.deletes.contains(&r) {
                    RowMark::Deleted
                } else {
                    RowMark::None
                };
                let updated = self.pending.updates.get(&r);
                GridRow {
                    values: visible.iter().map(|&c| self.cell(r, c)).collect(),
                    mark,
                    edited: visible
                        .iter()
                        .map(|c| updated.is_some_and(|m| m.contains_key(c)))
                        .collect(),
                }
            })
            .collect();
        let first = (self.page * PAGE) as usize + 1;
        let editable = self.editable();
        self.grid.update(cx, |g, cx| {
            g.set_editable(editable);
            g.set_rows(columns, rows, first);
            cx.notify();
        });
        cx.notify();
    }

    fn set_cell(&mut self, row: usize, col: usize, value: Value, cx: &mut Context<Self>) {
        if row < self.rows.len() {
            let original = &self.rows[row].values[col];
            let same =
                original.is_null() == value.is_null() && original.to_string() == value.to_string();
            let entry = self.pending.updates.entry(row).or_default();
            if same {
                entry.remove(&col);
                if entry.is_empty() {
                    self.pending.updates.remove(&row);
                }
            } else {
                entry.insert(col, value);
            }
        } else if let Some(insert) = self.pending.inserts.get_mut(row - self.rows.len()) {
            insert.insert(col, value);
        }
        self.refresh_grid(cx);
    }

    fn select(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if row >= self.display_rows() {
            return;
        }
        self.selected = Some(row);
        let values: Vec<Value> = (0..self.columns.len()).map(|c| self.cell(row, c)).collect();
        let mut subscriptions = Vec::new();
        let inputs = values
            .into_iter()
            .enumerate()
            .map(|(col, value)| {
                let text = if value.is_null() {
                    String::new()
                } else {
                    value.to_string()
                };
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("NULL")
                        .default_value(text)
                });
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this, input, event: &InputEvent, _, cx| {
                        if matches!(event, InputEvent::Change) && this.editable() {
                            let text = input.read(cx).value().to_string();
                            if let Some(row) = this.selected {
                                this.set_cell(row, col, parse_cell(&text), cx);
                            }
                        }
                    },
                ));
                input
            })
            .collect();
        self.inspector = Some(Inspector {
            row,
            inputs,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn insert_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        self.pending.inserts.push(BTreeMap::new());
        self.refresh_grid(cx);
        let row = self.display_rows() - 1;
        self.grid.update(cx, |g, cx| g.select(row, cx));
        self.select(row, window, cx);
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected.filter(|_| self.editable()) else {
            return;
        };
        if row >= self.rows.len() {
            self.pending.inserts.remove(row - self.rows.len());
            self.selected = None;
            self.inspector = None;
        } else if !self.pending.deletes.remove(&row) {
            self.pending.deletes.insert(row);
        }
        self.refresh_grid(cx);
    }

    fn discard(&mut self, cx: &mut Context<Self>) {
        self.pending = Pending::default();
        self.inspector = None;
        self.selected = None;
        self.error = None;
        self.refresh_grid(cx);
    }

    fn primary_key(&self, row: usize) -> Vec<ColumnValue> {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.primary_key)
            .map(|(i, c)| ColumnValue::new(c.name.clone(), self.rows[row].values[i].clone()))
            .collect()
    }

    /// Pending edits as `RowEdit`s: updates, then inserts, then deletes.
    fn edits(&self) -> Vec<RowEdit> {
        let column = |i: usize| self.columns[i].name.clone();
        let mut edits: Vec<RowEdit> = self
            .pending
            .updates
            .iter()
            .filter(|(r, _)| !self.pending.deletes.contains(r))
            .map(|(&r, changes)| RowEdit {
                table: self.table.clone(),
                primary_key: self.primary_key(r),
                kind: RowEditKind::Update {
                    changes: changes
                        .iter()
                        .map(|(&c, v)| ColumnValue::new(column(c), v.clone()))
                        .collect(),
                },
            })
            .collect();
        edits.extend(self.pending.inserts.iter().map(|values| {
            RowEdit {
                table: self.table.clone(),
                primary_key: Vec::new(),
                kind: RowEditKind::Insert {
                    values: values
                        .iter()
                        .map(|(&c, v)| ColumnValue::new(column(c), v.clone()))
                        .collect(),
                },
            }
        }));
        edits.extend(self.pending.deletes.iter().map(|&r| RowEdit {
            table: self.table.clone(),
            primary_key: self.primary_key(r),
            kind: RowEditKind::Delete,
        }));
        edits
    }

    pub(crate) fn set_safe_mode(&mut self, mode: SafeMode) {
        self.safe_mode = mode;
    }

    /// Saves, first asking when the connection confirms writes.
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_empty() || !self.editable() {
            return;
        }
        if self.safe_mode == SafeMode::ConfirmWrites {
            self.confirm_save = true;
            cx.notify();
            return;
        }
        self.save_now(cx);
    }

    fn save_now(&mut self, cx: &mut Context<Self>) {
        self.confirm_save = false;
        if self.pending.is_empty() || !self.editable() {
            return;
        }
        let edits = self.edits();
        let conn = self.conn.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.edit_rows() {
                    Some(e) => e.apply(&edits).await,
                    None => Err(Error::unsupported("edit rows")),
                }
            },
            |this, result, cx| {
                match result {
                    Ok(_) => this.reload(cx),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    /// Opens column stats under visible column `col`, or closes them.
    fn toggle_stats(&mut self, col: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = self.stats.take().is_some();
        self.grid.update(cx, |g, _| g.set_popover(None));
        let Some(column) = self
            .visible_columns()
            .get(col)
            .and_then(|i| self.columns.get(*i))
            .cloned()
        else {
            return;
        };
        if was_open || self.conn.column_stats().is_none() {
            cx.notify();
            return;
        }
        let (conn, table) = (self.conn.clone(), self.table.clone());
        let view = cx.new(|cx| ColumnStatsView::new(conn, table, column.clone(), cx));
        self._subscriptions.push(cx.subscribe_in(
            &view,
            window,
            move |this, _, action: &StatsAction, window, cx| {
                this.stats = None;
                this.grid.update(cx, |g, _| g.set_popover(None));
                match action {
                    StatsAction::Close => {}
                    StatsAction::Filter(value) => {
                        this.filters.clear();
                        let filter = Filter {
                            column: column.name.clone(),
                            op: FilterOp::Eq,
                            value: value.clone(),
                        };
                        this.add_filter(Some(filter), window, cx);
                        this.apply_filters(cx);
                    }
                    StatsAction::Chart => {
                        // ponytail: unquoted identifiers; fine for plain names, quote via the dialect if it bites.
                        let table = this.table.to_string();
                        let c = column.name.to_string();
                        cx.emit(OpenQuery(format!(
                        "SELECT {c}, count(*) AS n\nFROM {table}\nGROUP BY {c}\nORDER BY n DESC;"
                    )));
                    }
                }
                cx.notify();
            },
        ));
        self.grid
            .update(cx, |g, _| g.set_popover(Some((col, view.clone().into()))));
        self.stats = Some(view);
        cx.notify();
    }

    fn add_filter(&mut self, initial: Option<Filter>, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<SharedString> = self
            .columns
            .iter()
            .map(|c| c.name.to_string().into())
            .collect();
        let column_ix = initial
            .as_ref()
            .and_then(|f| self.columns.iter().position(|c| c.name == f.column))
            .unwrap_or(0);
        let op_ix = initial
            .as_ref()
            .and_then(|f| OPS.iter().position(|o| o.1 == f.op))
            .unwrap_or(0);
        let value = initial
            .as_ref()
            .map(|f| f.value.to_string())
            .unwrap_or_default();
        let column = cx.new(|_| Dropdown::new(names, column_ix).height(28.).mono());
        let op =
            cx.new(|_| Dropdown::new(OPS.iter().map(|o| o.0.into()).collect(), op_ix).height(28.));
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value")
                .default_value(value)
        });
        self._subscriptions.push(cx.subscribe_in(
            &value,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.apply_filters(cx);
                }
            },
        ));
        self.filters.push(FilterRow { column, op, value });
        cx.notify();
    }

    /// Fills column pickers once columns are known, keeping applied filters' columns.
    fn sync_filter_columns(&mut self, cx: &mut Context<Self>) {
        let names: Vec<SharedString> = self
            .columns
            .iter()
            .map(|c| c.name.to_string().into())
            .collect();
        for (i, row) in self.filters.iter().enumerate() {
            let selected = self
                .applied
                .get(i)
                .and_then(|f| self.columns.iter().position(|c| c.name == f.column))
                .unwrap_or_else(|| row.column.read(cx).selected());
            let names = names.clone();
            row.column.update(cx, |d, _| d.set_options(names, selected));
        }
    }

    fn apply_filters(&mut self, cx: &mut Context<Self>) {
        self.applied = self
            .filters
            .iter()
            .filter_map(|f| {
                let column = self.columns.get(f.column.read(cx).selected())?.name.clone();
                let op = OPS[f.op.read(cx).selected()].1;
                let text = f.value.read(cx).value().to_string();
                let needs_value = !matches!(op, FilterOp::IsNull | FilterOp::IsNotNull);
                (!needs_value || !text.is_empty()).then(|| Filter {
                    column,
                    op,
                    value: Value::Text(text),
                })
            })
            .collect();
        self.page = 0;
        self.reload(cx);
    }

    fn go(&mut self, delta: i64, cx: &mut Context<Self>) {
        let last = self.total.map_or(0, |t| t.saturating_sub(1) / PAGE);
        let next = (self.page as i64 + delta).clamp(0, last as i64) as u64;
        if next != self.page {
            self.page = next;
            self.reload(cx);
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let filters = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .flex_1()
            .min_w_0()
            .child(ui::icon("filter", 14., theme::FAINT))
            .children(self.filters.iter().enumerate().map(|(i, f)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().w(px(124.)).child(f.column.clone()))
                    .child(div().w(px(96.)).child(f.op.clone()))
                    .child(
                        div()
                            .w(px(160.))
                            .h(px(28.))
                            .rounded(px(6.))
                            .bg(c(theme::BG))
                            .border_1()
                            .border_color(c(theme::LINE))
                            .font_family(mono.clone())
                            .child(ui::input(&f.value, 12.5)),
                    )
                    .when(i > 0, |d| {
                        d.child(
                            ui::icon_button(("remove-filter", i), "close", 22., 13.).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.filters.remove(i);
                                    this.apply_filters(cx);
                                }),
                            ),
                        )
                    })
            }))
            .child(
                ui::secondary_button("apply", "Apply")
                    .on_click(cx.listener(|this, _, _, cx| this.apply_filters(cx))),
            )
            .child(
                ui::ghost_button("add-filter", "Add filter")
                    .flex_row_reverse()
                    .child(ui::icon("plus", 14., theme::MUTED))
                    .on_click(cx.listener(|this, _, window, cx| this.add_filter(None, window, cx))),
            );
        let actions = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .ml_auto()
            .child(
                ui::icon_button("insert-row", "plus", 28., 15.)
                    .on_click(cx.listener(|this, _, window, cx| this.insert_row(window, cx))),
            )
            .child(
                ui::icon_button("delete-rows", "trash", 28., 15.)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx))),
            )
            .child(
                ui::icon_button("reload", "refresh", 28., 15.)
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
            .child(ui::icon_button("export", "download", 28., 15.))
            .child(
                div()
                    .relative()
                    .child(
                        ui::icon_button("choose-columns", "columns", 28., 15.).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.columns_menu = !this.columns_menu;
                                cx.notify();
                            }),
                        ),
                    )
                    .children(self.columns_menu.then(|| self.render_columns_menu(cx))),
            );
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
                [("Data", true), ("Structure", false)],
                cx.listener(|this, ix: &usize, window, cx| {
                    if *ix == 1 {
                        this.open_structure(window, cx);
                    }
                }),
            ))
            .child(div().mx(px(4.)).w(px(1.)).h(px(20.)).bg(c(theme::LINE)))
            .child(filters)
            .child(actions)
    }

    fn open_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conn.describe().is_none() {
            return;
        }
        if self.structure.is_none() {
            let conn = self.conn.clone();
            let table = self.table.clone();
            let view = cx.new(|cx| StructureView::new(conn, table, cx));
            self._subscriptions.push(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &StructureEvent, _, cx| match event {
                    StructureEvent::ShowData => {
                        this.show_structure = false;
                        cx.notify();
                    }
                    StructureEvent::Changed => this.reload(cx),
                },
            ));
            self.structure = Some(view);
        }
        self.show_structure = true;
        cx.notify();
    }

    fn render_columns_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        deferred(
            anchored().snap_to_window().child(
                div()
                    .id("columns-menu")
                    .occlude()
                    .mt(px(30.))
                    .w(px(220.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .rounded(px(6.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .shadow_md()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.columns_menu = false;
                        cx.notify();
                    }))
                    .children(self.columns.iter().enumerate().map(|(i, col)| {
                        let shown = !self.hidden.contains(&i);
                        div()
                            .id(("toggle-col", i))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|d| d.bg(c(theme::SELECTION)))
                            .child(
                                div()
                                    .size(px(14.))
                                    .children(shown.then(|| ui::icon("check", 14., theme::TEXT))),
                            )
                            .child(
                                div()
                                    .font_family(mono.clone())
                                    .text_size(px(12.5))
                                    .child(SharedString::from(col.name.to_string())),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.hidden.remove(&i)
                                    && this.hidden.len() + 1 < this.columns.len()
                                {
                                    this.hidden.insert(i);
                                }
                                this.refresh_grid(cx);
                            }))
                    })),
            ),
        )
        .with_priority(1)
    }

    fn render_inspector(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let inspector = self.inspector.as_ref()?;
        let mono = theme::mono(cx);
        let row = inspector.row;
        let title = if row >= self.rows.len() {
            "NEW ROW".to_owned()
        } else {
            let key: String = self
                .primary_key(row)
                .iter()
                .map(|kv| format!(" · {} {}", kv.column.as_str().to_uppercase(), kv.value))
                .collect();
            format!("ROW {}{key}", self.page * PAGE + row as u64 + 1)
        };
        let mut fields = div().flex().flex_col().gap(px(12.)).p(px(12.));
        for (i, col) in self.columns.iter().enumerate() {
            let input = &inspector.inputs[i];
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .font_family(mono.clone())
                            .text_size(px(11.5))
                            .text_color(c(theme::MUTED))
                            .child(SharedString::from(col.name.to_string()))
                            .child(
                                div()
                                    .text_color(c(theme::FAINT))
                                    .child(SharedString::from(col.type_name.clone())),
                            ),
                    )
                    .child(
                        div()
                            .h(px(30.))
                            .rounded(px(6.))
                            .bg(c(theme::BG))
                            .border_1()
                            .border_color(c(if focused { theme::ACCENT } else { theme::LINE }))
                            .font_family(mono.clone())
                            .child(ui::input(input, 12.5)),
                    ),
            );
        }
        for (i, col) in self.columns.iter().enumerate() {
            let Some(target) = col.references.clone() else {
                continue;
            };
            let value = self.cell(row, i);
            if value.is_null() || value.to_string().is_empty() {
                continue;
            }
            let noun = col.name.as_str().trim_end_matches("_id").to_owned();
            let table = target.table.clone();
            let filter = Filter {
                column: target.column.clone(),
                op: FilterOp::Eq,
                value: value.clone(),
            };
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_size(px(11.5))
                            .text_color(c(theme::MUTED))
                            .child(SharedString::from(format!(
                                "{noun} → {}",
                                target.table.name
                            ))),
                    )
                    .child(
                        div()
                            .id(("fk-link", i))
                            .text_size(px(12.5))
                            .text_color(c(theme::ACCENT_TEXT))
                            .cursor_pointer()
                            .hover(|d| d.underline())
                            .child(SharedString::from(format!("Open {noun} #{value} →")))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(OpenTable {
                                    table: table.clone(),
                                    filter: filter.clone(),
                                });
                            })),
                    ),
            );
        }
        Some(
            div()
                .id("inspector")
                .flex()
                .flex_col()
                .flex_none()
                .w(px(INSPECTOR_WIDTH))
                .h_full()
                .overflow_y_scroll()
                .bg(c(theme::PANEL))
                .border_l_1()
                .border_color(c(theme::LINE))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(12.))
                        .py(px(10.))
                        .border_b_1()
                        .border_color(c(theme::LINE))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(c(theme::FAINT))
                                .child(SharedString::from(title)),
                        )
                        .child(div().ml_auto().child(
                            ui::icon_button("close-inspector", "close", 24., 15.).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.inspector = None;
                                    cx.notify();
                                }),
                            ),
                        )),
                )
                .child(fields),
        )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.rows.len() as u64;
        let from = self.page * PAGE + u64::from(shown > 0);
        let to = self.page * PAGE + shown;
        let sep = || div().text_color(c(theme::FAINT)).child("·");
        let edited = self.pending.edited_cells();
        let new = self.pending.inserts.len();
        let deleted = self.pending.deletes.len();
        let left = div()
            .flex()
            .items_center()
            .gap(px(14.))
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
            .when(!self.has_key() && !self.columns.is_empty(), |d| {
                d.child(sep()).child(
                    div()
                        .text_color(c(theme::FAINT))
                        .child("No primary key: read-only"),
                )
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(sep()).child(
                    div()
                        .text_color(c(theme::DELETED_TEXT))
                        .child(SharedString::from(e)),
                )
            });
        let right = (!self.pending.is_empty()).then(|| {
            div()
                .ml_auto()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .when(edited > 0, |d| {
                            d.child(ui::badge(
                                format!("{edited} edited"),
                                theme::EDITED_TEXT,
                                theme::EDITED_BG,
                            ))
                        })
                        .when(new > 0, |d| {
                            d.child(ui::badge(
                                format!("{new} new"),
                                theme::NEW_TEXT,
                                theme::NEW_BG,
                            ))
                        })
                        .when(deleted > 0, |d| {
                            d.child(ui::badge(
                                format!("{deleted} deleted"),
                                theme::DELETED_TEXT,
                                theme::DELETED_BG,
                            ))
                        }),
                )
                .child(
                    ui::ghost_button("discard", "Discard")
                        .on_click(cx.listener(|this, _, _, cx| this.discard(cx))),
                )
                .child(
                    ui::primary_button("save", "Save changes")
                        .child(ui::kbd("⌘S", cx).opacity(0.85))
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                )
        });
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
            .text_size(px(12.))
            .text_color(c(theme::MUTED))
            .child(left)
            .children(right)
    }
}

/// Text from an editor to a value: empty means `NULL`.
fn parse_cell(text: &str) -> Value {
    if text.is_empty() || text == "NULL" {
        Value::Null
    } else {
        Value::Text(text.to_owned())
    }
}

/// `'pending'::order_status` → `'pending'`, `nextval('x'::regclass)` → `nextval('x')`.
pub(crate) fn clean_default(raw: &str) -> String {
    let raw = raw
        .replace("::character varying", "::varchar")
        .replace("::timestamp with time zone", "::timestamptz")
        .replace("::timestamp without time zone", "::timestamp")
        .replace("::double precision", "::float8");
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            in_string = !in_string;
        }
        if !in_string && ch == ':' && chars.peek() == Some(&':') {
            chars.next();
            while chars
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                chars.next();
            }
            continue;
        }
        out.push(ch);
    }
    out
}

impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match (&self.error, self.columns.is_empty()) {
            (Some(error), true) => div()
                .p(px(16.))
                .text_size(px(13.))
                .text_color(c(theme::DELETED_TEXT))
                .child(SharedString::from(error.clone()))
                .into_any_element(),
            _ => self.grid.clone().into_any_element(),
        };
        div()
            .id("table-view")
            .key_context("TableView")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SaveChanges, _, cx| {
                match (&this.structure, this.show_structure) {
                    (Some(s), true) => s.update(cx, |s, cx| s.apply(cx)),
                    _ => this.save(cx),
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .map(
                |d| match self.structure.clone().filter(|_| self.show_structure) {
                    Some(structure) => d.child(structure),
                    None => d
                        .child(self.render_toolbar(cx))
                        .when(self.confirm_save, |d| {
                            d.child(ui::confirm_bar(
                                "This connection asks before writes. Save these changes?",
                                ui::primary_button("confirm-save", "Save")
                                    .on_click(cx.listener(|this, _, _, cx| this.save_now(cx))),
                                ui::ghost_button("cancel-save", "Cancel").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.confirm_save = false;
                                        cx.notify();
                                    },
                                )),
                            ))
                        })
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_h_0()
                                .child(div().flex_1().min_w_0().h_full().child(body))
                                .children(self.render_inspector(window, cx)),
                        )
                        .child(self.render_status(cx)),
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::clean_default;

    #[test]
    fn strips_casts_from_defaults() {
        assert_eq!(clean_default("'pending'::order_status"), "'pending'");
        assert_eq!(
            clean_default("nextval('orders_id_seq'::regclass)"),
            "nextval('orders_id_seq')"
        );
        assert_eq!(clean_default("'GBP'::bpchar"), "'GBP'");
        assert_eq!(clean_default("now()"), "now()");
        assert_eq!(clean_default("'a::b'::character varying"), "'a::b'");
    }
}
