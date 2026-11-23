//! One open connection: schema sidebar, document tabs and the active document.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyView, Context, Entity, FontWeight, SharedString, Subscription, Task, Window, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use traits::Connection;
use types::{
    ConnectionId, Dashboard, DbObject, Error, Filter, KeyBytes, KeyDirectory, ObjectKind, SafeMode,
    SchemaName, TableRef, Tag,
};

use crate::dashboard_view::{AddChart, DashboardView};
use crate::kv_view::{KeyEvent, KeyTree, KeyView, MemoryPanel, OpenKey};
use crate::range_view::{DirectoryTree, OpenDirectory, RangeView, ShowSystem};
use crate::sql_editor::{Pinned, SqlEditor};
use crate::table_view::{OpenQuery, OpenTable, TableView};
use crate::theme::{self, c};
use crate::{runtime, state, ui};

/// Sidebar sections, in order.
const SECTIONS: [(ObjectKind, &str); 4] = [
    (ObjectKind::Table, "Tables"),
    (ObjectKind::View, "Views"),
    (ObjectKind::Function, "Functions"),
    (ObjectKind::Enum, "Enums"),
];

/// An open document tab.
pub(crate) enum Doc {
    Table(Entity<TableView>),
    Query(Entity<SqlEditor>),
    Key(Entity<KeyView>),
    Dashboard(Entity<DashboardView>),
    Range(Entity<RangeView>),
}

impl Doc {
    fn view(&self) -> AnyView {
        match self {
            Doc::Table(v) => v.clone().into(),
            Doc::Query(v) => v.clone().into(),
            Doc::Key(v) => v.clone().into(),
            Doc::Dashboard(v) => v.clone().into(),
            Doc::Range(v) => v.clone().into(),
        }
    }
}

/// A live connection and its workspace.
pub(crate) struct Session {
    pub(crate) name: SharedString,
    pub(crate) tag: Tag,
    pub(crate) safe_mode: SafeMode,
    pub(crate) conn: Arc<dyn Connection>,
    id: ConnectionId,
    queries: usize,
    schema: Option<SchemaName>,
    schemas: Vec<SchemaName>,
    objects: Vec<DbObject>,
    collapsed: Vec<ObjectKind>,
    filter: Entity<InputState>,
    docs: Vec<Doc>,
    dashboards: Vec<Dashboard>,
    active: Option<usize>,
    error: Option<String>,
    keys: Option<Entity<KeyTree>>,
    directories: Option<Entity<DirectoryTree>>,
    show_system: bool,
    memory: Option<Entity<MemoryPanel>>,
    _loading: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Session {
    pub(crate) fn new(
        name: SharedString,
        tag: Tag,
        safe_mode: SafeMode,
        id: ConnectionId,
        conn: Arc<dyn Connection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter tables"));
        let subscription = cx.subscribe_in(&filter, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut session = Self {
            name,
            tag,
            safe_mode,
            conn,
            id,
            queries: 0,
            schema: None,
            schemas: Vec::new(),
            objects: Vec::new(),
            collapsed: vec![ObjectKind::View, ObjectKind::Function, ObjectKind::Enum],
            filter,
            docs: Vec::new(),
            dashboards: state::dashboards(cx).list(id).unwrap_or_default(),
            active: None,
            error: None,
            keys: None,
            directories: None,
            show_system: false,
            memory: None,
            _loading: None,
            _subscriptions: vec![subscription],
        };
        if session.conn.key_directories().is_some() {
            let (conn, name) = (session.conn.clone(), session.name.clone());
            let tree = cx.new(|cx| DirectoryTree::new(conn, name, window, cx));
            session._subscriptions.push(cx.subscribe_in(
                &tree,
                window,
                |this, _, e: &OpenDirectory, window, cx| {
                    this.open_range(e.0.clone(), window, cx);
                },
            ));
            session
                ._subscriptions
                .push(cx.subscribe(&tree, |this, _, e: &ShowSystem, cx| {
                    this.show_system = e.0;
                    for doc in &this.docs {
                        if let Doc::Range(v) = doc {
                            v.update(cx, |v, cx| v.set_include_system(e.0, cx));
                        }
                    }
                }));
            session.directories = Some(tree);
        } else if session.conn.key_value().is_some() {
            let tree = cx.new(|cx| KeyTree::new(session.conn.clone(), window, cx));
            session._subscriptions.push(cx.subscribe_in(
                &tree,
                window,
                |this, _, e: &OpenKey, window, cx| {
                    this.open_key(e.0.clone(), window, cx);
                },
            ));
            session.keys = Some(tree);
        }
        // The range browser uses the right side for the selected value.
        if session.conn.memory_stats().is_some() && session.directories.is_none() {
            let conn = session.conn.clone();
            session.memory = Some(cx.new(|cx| MemoryPanel::new(conn, cx)));
        }
        if session.conn.browse().is_some() {
            session.refresh(cx);
        }
        session
    }

    /// The engine label shown on the connection tab, e.g. `PostgreSQL 16`.
    pub(crate) fn engine(&self) -> SharedString {
        // Drivers report the version as it should be shown (`PostgreSQL 16`, `MySQL 8.0`).
        self.conn.info().server_version.clone().into()
    }

    /// Database name for the breadcrumb; empty when the engine has none.
    pub(crate) fn database(&self) -> String {
        self.conn.info().database.clone().unwrap_or_default()
    }

    /// Current schema, if the engine has schemas and one is loaded.
    pub(crate) fn schema(&self) -> Option<&SchemaName> {
        self.schema.as_ref()
    }

    /// Schemas to pick from in the breadcrumb.
    pub(crate) fn schemas(&self) -> &[SchemaName] {
        &self.schemas
    }

    /// Switches schema and reloads the sidebar.
    pub(crate) fn set_schema(&mut self, schema: SchemaName, cx: &mut Context<Self>) {
        self.schema = Some(schema);
        self.refresh(cx);
    }

    /// Reloads schemas and objects.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let wanted = self.schema.clone();
        self._loading = Some(runtime::spawn_db(
            cx,
            async move {
                let browse = conn.browse().ok_or_else(|| Error::unsupported("browse"))?;
                let objects = async {
                    let schema = match wanted {
                        Some(s) => Some(s),
                        None => browse.default_schema().await?,
                    };
                    let objects = match &schema {
                        Some(s) => browse.objects(s).await?,
                        None => Vec::new(),
                    };
                    Ok::<_, Error>((schema, objects))
                };
                let (objects, schemas) = futures::join!(objects, browse.schemas());
                let (schema, objects) = objects?;
                let schemas = schemas.unwrap_or_default();
                Ok::<_, Error>((schema, objects, schemas))
            },
            |this, result, cx| {
                match result {
                    Ok((schema, objects, schemas)) => {
                        this.schema = schema;
                        this.objects = objects;
                        this.schemas = schemas;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    /// Opens (or focuses) a table document. A filter always opens a new tab.
    pub(crate) fn open_table(
        &mut self,
        table: TableRef,
        filter: Option<Filter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reuse = filter.is_none();
        self.open_table_with(table, filter, reuse, window, cx);
    }

    /// Opens a table in a new tab even if one is open.
    pub(crate) fn open_table_in_new_tab(
        &mut self,
        table: TableRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_table_with(table, None, false, window, cx);
    }

    fn open_table_with(
        &mut self,
        table: TableRef,
        filter: Option<Filter>,
        reuse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = reuse.then(|| {
            self.docs.iter().position(|d| match d {
                Doc::Table(v) => v.read(cx).table == table,
                Doc::Query(_) | Doc::Key(_) | Doc::Dashboard(_) | Doc::Range(_) => false,
            })
        });
        let ix = match existing.flatten() {
            Some(ix) => ix,
            None => {
                let conn = self.conn.clone();
                let view = cx.new(|cx| TableView::new(conn, table, filter, window, cx));
                let mode = self.safe_mode;
                view.update(cx, |v, _| v.set_safe_mode(mode));
                self._subscriptions.push(cx.subscribe_in(
                    &view,
                    window,
                    |this, _, event: &OpenTable, window, cx| {
                        this.open_table(
                            event.table.clone(),
                            Some(event.filter.clone()),
                            window,
                            cx,
                        );
                    },
                ));
                self._subscriptions.push(cx.subscribe_in(
                    &view,
                    window,
                    |this, _, event: &OpenQuery, window, cx| {
                        this.open_query(event.0.clone(), window, cx)
                    },
                ));
                self.docs.push(Doc::Table(view));
                self.docs.len() - 1
            }
        };
        self.active = Some(ix);
        cx.notify();
    }

    fn reload_dashboards(&mut self, cx: &mut Context<Self>) {
        self.dashboards = state::dashboards(cx).list(self.id).unwrap_or_default();
        for doc in &self.docs {
            if let Doc::Dashboard(v) = doc {
                v.update(cx, |v, cx| v.reload(cx));
            }
        }
        cx.notify();
    }

    /// Tables and views from the sidebar, for the command palette.
    pub(crate) fn objects(&self) -> &[DbObject] {
        &self.objects
    }

    pub(crate) fn dashboards(&self) -> &[Dashboard] {
        &self.dashboards
    }

    pub(crate) fn id(&self) -> ConnectionId {
        self.id
    }

    /// Opens (or focuses) a directory's range document.
    pub(crate) fn open_range(
        &mut self,
        dir: KeyDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self
            .docs
            .iter()
            .position(|d| matches!(d, Doc::Range(v) if v.read(cx).dir == dir));
        let ix = match existing {
            Some(ix) => ix,
            None => {
                let (conn, system) = (self.conn.clone(), self.show_system);
                let view = cx.new(|cx| RangeView::new(conn, dir, system, window, cx));
                self.docs.push(Doc::Range(view));
                self.docs.len() - 1
            }
        };
        self.active = Some(ix);
        cx.notify();
    }

    /// Opens (or focuses) a dashboard tab.
    pub(crate) fn open_dashboard(
        &mut self,
        dashboard: Dashboard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self.docs.iter().position(
            |d| matches!(d, Doc::Dashboard(v) if v.read(cx).dashboard.id == dashboard.id),
        );
        let ix = match existing {
            Some(ix) => ix,
            None => {
                let (conn, name) = (self.conn.clone(), self.name.clone());
                let view = cx.new(|cx| DashboardView::new(conn, dashboard, name, window, cx));
                self._subscriptions.push(cx.subscribe_in(
                    &view,
                    window,
                    |this, _, _: &AddChart, window, cx| this.open_query(String::new(), window, cx),
                ));
                self.docs.push(Doc::Dashboard(view));
                self.docs.len() - 1
            }
        };
        self.active = Some(ix);
        cx.notify();
    }

    /// Opens a new query tab with `sql` in the editor.
    pub(crate) fn open_query(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        self.queries += 1;
        let title: SharedString = format!("Query {}", self.queries).into();
        let label: SharedString = match &self.conn.info().database {
            Some(db) => format!("{} · {db}", self.name).into(),
            None => self.name.clone(),
        };
        let (conn, id, tag) = (self.conn.clone(), self.id, self.tag);
        let editor = cx.new(|cx| SqlEditor::new(conn, id, label, tag, title, sql, window, cx));
        let mode = self.safe_mode;
        editor.update(cx, |e, _| e.set_safe_mode(mode));
        self._subscriptions
            .push(cx.subscribe(&editor, |this, _, _: &Pinned, cx| {
                this.reload_dashboards(cx)
            }));
        self.docs.push(Doc::Query(editor));
        self.active = Some(self.docs.len() - 1);
        cx.notify();
    }

    /// Opens (or focuses) a key document.
    pub(crate) fn open_key(&mut self, key: KeyBytes, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self
            .docs
            .iter()
            .position(|d| matches!(d, Doc::Key(v) if v.read(cx).key == key));
        let ix = match existing {
            Some(ix) => ix,
            None => {
                let conn = self.conn.clone();
                let view = cx.new(|cx| KeyView::new(conn, key, window, cx));
                self._subscriptions.push(cx.subscribe_in(
                    &view,
                    window,
                    |this, view, e: &KeyEvent, _, cx| {
                        match e {
                            KeyEvent::Deleted(key) => {
                                if let Some(ix) = this
                                    .docs
                                    .iter()
                                    .position(|d| matches!(d, Doc::Key(v) if v == view))
                                {
                                    this.close_doc(ix, cx);
                                }
                                if let Some(tree) = &this.keys {
                                    tree.update(cx, |t, cx| t.forget(key, cx));
                                }
                            }
                            KeyEvent::Renamed => {
                                if let Some(tree) = &this.keys {
                                    tree.update(cx, |t, cx| t.rescan(cx));
                                }
                            }
                        }
                        cx.notify();
                    },
                ));
                self.docs.push(Doc::Key(view));
                self.docs.len() - 1
            }
        };
        self.active = Some(ix);
        cx.notify();
    }

    fn close_doc(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.docs.len() {
            if let Doc::Query(v) = &self.docs[ix] {
                v.update(cx, |v, cx| v.cancel(cx));
            }
            self.docs.remove(ix);
            self.active = match self.active {
                _ if self.docs.is_empty() => None,
                Some(a) if a >= ix && a > 0 => Some(a - 1),
                other => other,
            };
            cx.notify();
        }
    }

    fn selected_table(&self, cx: &Context<Self>) -> Option<TableRef> {
        match self.docs.get(self.active?)? {
            Doc::Table(v) => Some(v.read(cx).table.clone()),
            Doc::Query(_) | Doc::Key(_) | Doc::Dashboard(_) | Doc::Range(_) => None,
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.filter.read(cx).value().to_lowercase();
        let selected = self.selected_table(cx);
        let header = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .when(self.conn.edit_rows().is_none(), |d| {
                let engine = self
                    .conn
                    .info()
                    .server_version
                    .split_whitespace()
                    .next()
                    .unwrap_or("this database")
                    .to_owned();
                d.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .p(px(10.))
                        .rounded(px(6.))
                        .bg(c(theme::EDITED_BG))
                        .text_size(px(12.5))
                        .text_color(c(theme::EDITED_TEXT))
                        .child(ui::icon("info", 13., theme::EDITED_TEXT))
                        .child(div().flex_1().min_w_0().child(SharedString::from(
                            if self.safe_mode == SafeMode::ReadOnly {
                                "Read-only: this connection cannot write by any path.".to_owned()
                            } else {
                                format!("Read-mostly: cell edits are off for {engine}. Inserts still work.")
                            },
                        ))),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(0.))
                    .h(px(30.))
                    .pl(px(9.))
                    .pr(px(4.))
                    .rounded(px(10.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .child(ui::icon("search", 13., theme::FAINT))
                    .child(div().flex_1().child(ui::input(&self.filter, 13.))),
            );

        let mut nav = div()
            .id("objects")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(px(2.))
            .px(px(6.))
            .py(px(8.));
        for (kind, label) in SECTIONS {
            let items: Vec<&DbObject> = self
                .objects
                .iter()
                .filter(|o| {
                    let shown = if kind == ObjectKind::View {
                        matches!(o.kind, ObjectKind::View | ObjectKind::MaterializedView)
                    } else {
                        o.kind == kind
                    };
                    shown && o.table.name.as_str().to_lowercase().contains(&filter)
                })
                .collect();
            let open = !self.collapsed.contains(&kind);
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("section-{label}")))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(8.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::MUTED))
                    .child(ui::icon(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        13.,
                        theme::MUTED,
                    ))
                    .child(label.to_uppercase())
                    .child(
                        div()
                            .ml_auto()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(c(theme::FAINT))
                            .child(SharedString::from(items.len().to_string())),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        match this.collapsed.iter().position(|k| *k == kind) {
                            Some(i) => {
                                this.collapsed.remove(i);
                            }
                            None => this.collapsed.push(kind),
                        }
                        cx.notify();
                    })),
            );
            if !open {
                continue;
            }
            for object in items {
                let table = object.table.clone();
                let is_selected = selected.as_ref() == Some(&object.table);
                let openable = matches!(
                    object.kind,
                    ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
                );
                let row = div()
                    .id(SharedString::from(format!("obj-{}", object.table)))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .flex_none()
                    .pl(px(26.))
                    .pr(px(10.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_color(c(if is_selected {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    }))
                    .map(|d| {
                        if is_selected {
                            d.bg(c(theme::BG)).shadow(ui::soft_shadow(1., 2., 0.08))
                        } else {
                            d.hover(|d| d.bg(c(theme::RAISED)))
                        }
                    })
                    .child(ui::icon(
                        if openable { "table" } else { "code" },
                        14.,
                        if is_selected {
                            theme::ACCENT_TEXT
                        } else {
                            theme::FAINT
                        },
                    ))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .child(SharedString::from(object.table.name.to_string())),
                    );
                nav = nav.child(if openable {
                    row.on_click(cx.listener(move |this, _, window, cx| {
                        this.open_table(table.clone(), None, window, cx)
                    }))
                } else {
                    row
                });
            }
        }

        if !self.dashboards.is_empty() {
            nav = nav.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(8.))
                    .mt(px(6.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::MUTED))
                    .child(ui::icon("chevron-down", 13., theme::MUTED))
                    .child("DASHBOARDS")
                    .child(
                        div()
                            .ml_auto()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(c(theme::FAINT))
                            .child(SharedString::from(self.dashboards.len().to_string())),
                    ),
            );
            for (i, d) in self.dashboards.iter().enumerate() {
                let dashboard = d.clone();
                nav = nav.child(
                    div()
                        .id(("dashboard", i))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(34.))
                        .flex_none()
                        .pl(px(26.))
                        .pr(px(10.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .text_color(c(theme::MUTED))
                        .hover(|d| d.bg(c(theme::RAISED)))
                        .child(ui::icon("chart", 14., theme::FAINT))
                        .child(SharedString::from(d.title.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_dashboard(dashboard.clone(), window, cx)
                        })),
                );
            }
        }

        let footer =
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(8.))
                .border_t_1()
                .border_color(c(theme::LINE))
                .text_size(px(12.))
                .text_color(c(theme::FAINT))
                .child(div().size(px(7.)).flex_none().rounded_full().bg(c(
                    if self.error.is_some() {
                        theme::DELETED_TEXT
                    } else {
                        theme::CONNECTED
                    },
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(match &self.error {
                            Some(e) => e.clone(),
                            None => format!("Connected · {}", self.conn.info().address),
                        })),
                )
                .child(
                    div().ml_auto().child(
                        ui::icon_button("refresh-schema", "refresh", 24., 14.)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
                );

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(272.))
            .h_full()
            .pt(px(4.))
            .pl(px(10.))
            .pr(px(6.))
            .child(header)
            .child(nav)
            .child(footer)
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(14.))
            .py(px(10.))
            .border_b_1()
            .border_color(c(theme::LINE))
            .children(self.docs.iter().enumerate().map(|(ix, doc)| {
                let active = self.active == Some(ix);
                let (icon, label) = match doc {
                    Doc::Table(v) => ("table", v.read(cx).table.name.to_string()),
                    Doc::Query(v) => ("code", v.read(cx).title.to_string()),
                    Doc::Key(v) => ("table", v.read(cx).key.to_display()),
                    Doc::Dashboard(v) => ("chart", v.read(cx).dashboard.title.clone()),
                    Doc::Range(v) => ("folder", v.read(cx).dir.path.join("/")),
                };
                div()
                    .id(("doc-tab", ix))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(30.))
                    .pl(px(12.))
                    .pr(px(4.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(c(theme::RAISED)))
                    .text_color(c(if active { theme::TEXT } else { theme::MUTED }))
                    .child(ui::icon(
                        icon,
                        14.,
                        if active {
                            theme::ACCENT_TEXT
                        } else {
                            theme::FAINT
                        },
                    ))
                    .child(div().text_size(px(12.5)).child(SharedString::from(label)))
                    .child(
                        ui::icon_button(("close-doc", ix), "close", 22., 15.).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_doc(ix, cx);
                            }),
                        ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = Some(ix);
                        cx.notify();
                    }))
            }))
            .when(self.conn.query().is_some(), |d| {
                d.child(ui::icon_button("new-query", "plus", 28., 15.).on_click(
                    cx.listener(|this, _, window, cx| this.open_query(String::new(), window, cx)),
                ))
            })
    }
}

impl Render for Session {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("Session::render").entered();
        let content = self.active.and_then(|ix| self.docs.get(ix)).map(Doc::view);
        let sidebar = match (&self.directories, &self.keys) {
            (Some(tree), _) => tree.clone().into_any_element(),
            (None, Some(tree)) => tree.clone().into_any_element(),
            (None, None) => self.render_sidebar(cx).into_any_element(),
        };
        div().flex().size_full().min_h_0().child(sidebar).child(
            // The floating white panel: inset from the right and bottom window edges.
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .mr(px(12.))
                .mb(px(12.))
                .ml(px(4.))
                .rounded(px(14.))
                .overflow_hidden()
                .border_1()
                .border_color(c(theme::LINE))
                .shadow(ui::soft_shadow(1., 2., 0.04))
                .bg(c(theme::BG))
                .child(self.render_tabs(cx))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .child(div().flex_1().min_w_0().h_full().children(content))
                        .children(self.memory.clone()),
                ),
        )
    }
}
