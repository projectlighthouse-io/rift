//! One open connection: schema sidebar, document tabs and the active document.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyView, Context, Entity, FontWeight, SharedString, Subscription, Task, Window, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use traits::Connection;
use types::{
    ConnectionId, DbObject, Error, Filter, KeyBytes, ObjectKind, SchemaName, TableRef, Tag,
};

use crate::kv_view::{KeyEvent, KeyTree, KeyView, MemoryPanel, OpenKey};
use crate::sql_editor::SqlEditor;
use crate::table_view::{OpenTable, TableView};
use crate::theme::{self, c};
use crate::{runtime, ui};

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
}

impl Doc {
    fn view(&self) -> AnyView {
        match self {
            Doc::Table(v) => v.clone().into(),
            Doc::Query(v) => v.clone().into(),
            Doc::Key(v) => v.clone().into(),
        }
    }
}

/// A live connection and its workspace.
pub(crate) struct Session {
    pub(crate) name: SharedString,
    pub(crate) tag: Tag,
    pub(crate) read_only: bool,
    pub(crate) conn: Arc<dyn Connection>,
    id: ConnectionId,
    queries: usize,
    schema: Option<SchemaName>,
    schemas: Vec<SchemaName>,
    schema_menu: bool,
    objects: Vec<DbObject>,
    collapsed: Vec<ObjectKind>,
    filter: Entity<InputState>,
    docs: Vec<Doc>,
    active: Option<usize>,
    error: Option<String>,
    keys: Option<Entity<KeyTree>>,
    memory: Option<Entity<MemoryPanel>>,
    _loading: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Session {
    pub(crate) fn new(
        name: SharedString,
        tag: Tag,
        read_only: bool,
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
            read_only,
            conn,
            id,
            queries: 0,
            schema: None,
            schemas: Vec::new(),
            schema_menu: false,
            objects: Vec::new(),
            collapsed: vec![ObjectKind::View, ObjectKind::Function, ObjectKind::Enum],
            filter,
            docs: Vec::new(),
            active: None,
            error: None,
            keys: None,
            memory: None,
            _loading: None,
            _subscriptions: vec![subscription],
        };
        if session.conn.key_value().is_some() {
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
        if session.conn.memory_stats().is_some() {
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
        // `PostgreSQL 16.4` → `PostgreSQL 16`.
        let version = &self.conn.info().server_version;
        match version.rsplit_once(' ') {
            Some((name, number)) => {
                format!("{name} {}", number.split('.').next().unwrap_or(number)).into()
            }
            None => version.clone().into(),
        }
    }

    /// Reloads schemas and objects.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let wanted = self.schema.clone();
        self._loading = Some(runtime::spawn_db(
            cx,
            async move {
                let browse = conn.browse().ok_or_else(|| Error::unsupported("browse"))?;
                let schema = match wanted {
                    Some(s) => Some(s),
                    None => browse.default_schema().await?,
                };
                let objects = match &schema {
                    Some(s) => browse.objects(s).await?,
                    None => Vec::new(),
                };
                let schemas = browse.schemas().await.unwrap_or_default();
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
        let existing = filter.is_none().then(|| {
            self.docs.iter().position(|d| match d {
                Doc::Table(v) => v.read(cx).table == table,
                Doc::Query(_) | Doc::Key(_) => false,
            })
        });
        let ix = match existing.flatten() {
            Some(ix) => ix,
            None => {
                let conn = self.conn.clone();
                let view = cx.new(|cx| TableView::new(conn, table, filter, window, cx));
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
                self.docs.push(Doc::Table(view));
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
            Doc::Query(_) | Doc::Key(_) => None,
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let filter = self.filter.read(cx).value().to_lowercase();
        let selected = self.selected_table(cx);
        let database = self.conn.info().database.clone().unwrap_or_default();
        let schema = self
            .schema
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_default();

        let header = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .border_b_1()
            .border_color(c(theme::LINE))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::FAINT))
                    .child("DATABASE · SCHEMA"),
            )
            .child(
                div()
                    .id("schema-picker")
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.schema_menu = !this.schema_menu;
                        cx.notify();
                    }))
                    .children(self.schema_menu.then(|| self.render_schema_menu(cx)))
                    .bg(c(theme::RAISED))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .text_size(px(13.))
                    .child(ui::icon("database", 14., theme::ACCENT_TEXT))
                    // Engines without a schema level (MySQL) report the database as the schema.
                    .when(database != schema && !database.is_empty(), |d| {
                        d.child(
                            div()
                                .font_family(mono.clone())
                                .text_size(px(12.5))
                                .child(SharedString::from(database.clone())),
                        )
                        .child(div().text_color(c(theme::FAINT)).child("/"))
                    })
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_size(px(12.5))
                            .child(SharedString::from(schema)),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .child(ui::icon("chevron-down", 14., theme::FAINT)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(0.))
                    .h(px(30.))
                    .pl(px(9.))
                    .pr(px(4.))
                    .rounded(px(6.))
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
                    .h(px(28.))
                    .pl(px(26.))
                    .pr(px(10.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_color(c(if is_selected {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    }))
                    .when(is_selected, |d| d.bg(c(theme::SELECTION)))
                    .hover(|d| d.bg(c(theme::SELECTION)))
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
                            .font_family(mono.clone())
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

        let footer = div()
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
            .child(
                div()
                    .size(px(7.))
                    .rounded(px(2.))
                    .bg(c(if self.error.is_some() {
                        theme::DELETED_TEXT
                    } else {
                        theme::tag_color(Tag::Green)
                    })),
            )
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
            .w(px(249.))
            .h_full()
            .bg(c(theme::PANEL))
            .border_r_1()
            .border_color(c(theme::LINE))
            .child(header)
            .child(nav)
            .child(footer)
    }

    fn render_schema_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        gpui::deferred(
            gpui::anchored().snap_to_window().child(
                div()
                    .id("schema-menu")
                    .occlude()
                    .mt(px(34.))
                    .w(px(224.))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .rounded(px(6.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .shadow_md()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.schema_menu = false;
                        cx.notify();
                    }))
                    .children(self.schemas.iter().enumerate().map(|(i, schema)| {
                        let picked = schema.clone();
                        let current = self.schema.as_ref() == Some(schema);
                        div()
                            .id(("schema-option", i))
                            .flex()
                            .items_center()
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .font_family(mono.clone())
                            .text_size(px(12.5))
                            .when(current, |d| d.bg(c(theme::SELECTION)))
                            .hover(|d| d.bg(c(theme::SELECTION)))
                            .child(SharedString::from(schema.to_string()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.schema = Some(picked.clone());
                                this.schema_menu = false;
                                this.refresh(cx);
                            }))
                    })),
            ),
        )
        .with_priority(1)
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .px(px(8.))
            .h(px(37.))
            .bg(c(theme::PANEL))
            .border_b_1()
            .border_color(c(theme::LINE))
            .children(self.docs.iter().enumerate().map(|(ix, doc)| {
                let active = self.active == Some(ix);
                let (icon, label, is_table) = match doc {
                    Doc::Table(v) => ("table", v.read(cx).table.name.to_string(), true),
                    Doc::Query(v) => ("code", v.read(cx).title.to_string(), false),
                    Doc::Key(v) => ("table", v.read(cx).key.to_display(), true),
                };
                div()
                    .id(("doc-tab", ix))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(36.))
                    .pl(px(12.))
                    .pr(px(6.))
                    .cursor_pointer()
                    .border_b_2()
                    .border_color(if active {
                        c(theme::ACCENT)
                    } else {
                        gpui::transparent_black().into()
                    })
                    .bg(if active {
                        c(theme::BG)
                    } else {
                        gpui::transparent_black().into()
                    })
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
                    .child(
                        div()
                            .text_size(px(12.5))
                            .when(is_table, |d| d.font_family(mono.clone()))
                            .child(SharedString::from(label)),
                    )
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
        let content = self.active.and_then(|ix| self.docs.get(ix)).map(Doc::view);
        let sidebar = match &self.keys {
            Some(tree) => tree.clone().into_any_element(),
            None => self.render_sidebar(cx).into_any_element(),
        };
        div().flex().size_full().min_h_0().child(sidebar).child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(ui::dot_grid())
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
