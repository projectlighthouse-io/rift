//! The window root: breadcrumb top bar (connection switcher, database, schema), then the active
//! session or the start screen (connections list / connection dialog).

use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FontWeight, SharedString, Subscription, Window, canvas, div, px,
};
use types::{ObjectKind, SafeMode, SavedConnection, Tag};

use crate::actions::{OpenPalette, ToggleSwitcher};
use crate::connection_dialog::{ConnectionDialog, DialogEvent};
use crate::connections::{ConnectionsEvent, ConnectionsScreen};
use crate::palette::{self, Item, Palette, PaletteEvent, Target};
use crate::session::Session;
use crate::theme::{self, c};
use crate::{state, ui};

/// Frames slower than this are logged in debug builds.
const FRAME_BUDGET_MS: u128 = 16;

/// `RIFT_FRAME_LOG=1` enables slow-frame warnings in release builds.
fn frame_log_enabled() -> bool {
    std::env::var_os("RIFT_FRAME_LOG").is_some()
}

enum Start {
    List(Entity<ConnectionsScreen>),
    Dialog(Entity<ConnectionDialog>),
}

/// Root view.
pub(crate) struct Workspace {
    sessions: Vec<Entity<Session>>,
    active: Option<usize>,
    start: Option<Start>,
    _start_subscription: Option<Subscription>,
    palette: Option<(Entity<Palette>, Subscription)>,
    /// Status of a backup or restore, shown as a toast: text and whether it is an error.
    notice: Option<(String, bool)>,
    _job: Option<gpui::Task<()>>,
    /// Root focus, so ⌘K works before anything else has focus.
    focus: gpui::FocusHandle,
    /// Connection switcher dropdown is open.
    switcher: bool,
    /// Switcher row whose right-click menu (Disconnect) is showing.
    switcher_menu: Option<usize>,
    /// Schema crumb dropdown is open.
    schema_menu: bool,
}

impl Workspace {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut workspace = Self {
            sessions: Vec::new(),
            active: None,
            start: None,
            _start_subscription: None,
            palette: None,
            notice: None,
            _job: None,
            focus: cx.focus_handle(),
            switcher: false,
            switcher_menu: None,
            schema_menu: false,
        };
        window.focus(&workspace.focus, cx);
        workspace.show_list(window, cx);
        // Dev check: `RIFT_DEV_CONNECT=a,b` connects to these saved connections at startup.
        if let Ok(names) = std::env::var("RIFT_DEV_CONNECT") {
            let saved = state::manager(cx).list().unwrap_or_default();
            for name in names.split(',') {
                if let Some(conn) = saved.iter().find(|s| s.name == name.trim()) {
                    workspace.connect(conn.clone(), window, cx);
                }
            }
        }
        workspace
    }

    /// Everything the palette searches, gathered when it opens.
    fn palette_items(&self, cx: &gpui::App) -> Vec<Item> {
        let mut items = Vec::new();
        let mut open = Vec::new();
        for (i, session) in self.sessions.iter().enumerate() {
            let s = session.read(cx);
            open.push(s.id());
            let openable = |k: ObjectKind| {
                matches!(
                    k,
                    ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
                )
            };
            for o in s.objects().iter().filter(|o| openable(o.kind)) {
                items.push(palette::object_item(i, &s.name, o.kind, o.table.clone()));
            }
            if s.conn.backup().is_some() {
                items.push(Item::new(
                    "download",
                    format!("Back up {}…", s.name),
                    "backup".into(),
                    false,
                    Target::Backup { session: i },
                ));
                if s.safe_mode != SafeMode::ReadOnly {
                    items.push(Item::new(
                        "refresh",
                        format!("Restore {} from backup…", s.name),
                        "backup".into(),
                        false,
                        Target::Restore { session: i },
                    ));
                }
            }
            for d in s.dashboards() {
                items.push(Item::new(
                    "chart",
                    d.title.clone(),
                    format!("{} · dashboard", s.name),
                    false,
                    Target::Dashboard {
                        session: i,
                        dashboard: d.clone(),
                    },
                ));
            }
        }
        for entry in state::history(cx).recent(None, 200).unwrap_or_default() {
            let Some(session) = open.iter().position(|id| *id == entry.connection) else {
                continue;
            };
            items.push(Item::new(
                "code",
                palette::history_label(&entry.sql),
                format!("history · {}", palette::day_label(entry.ran_at)),
                false,
                Target::Query {
                    session,
                    sql: entry.sql,
                },
            ));
        }
        for saved in state::manager(cx).list().unwrap_or_default() {
            if !open.contains(&saved.id) {
                items.push(Item::new(
                    "database",
                    format!("Connect to {}", saved.name),
                    "connection".into(),
                    false,
                    Target::Connect(saved),
                ));
            }
        }
        items
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.palette_items(cx);
        let palette = cx.new(|cx| Palette::new(items, window, cx));
        let subscription = cx.subscribe_in(
            &palette,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.palette = None;
                if let PaletteEvent::Open { target, new_tab } = event {
                    this.open_target((**target).clone(), *new_tab, window, cx);
                }
                cx.notify();
            },
        );
        self.palette = Some((palette, subscription));
        cx.notify();
    }

    fn open_target(
        &mut self,
        target: Target,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session_ix = match &target {
            Target::Table { session, .. }
            | Target::Dashboard { session, .. }
            | Target::Query { session, .. } => *session,
            Target::Connect(saved) => {
                self.connect(saved.clone(), window, cx);
                return;
            }
            Target::Backup { session } => {
                self.backup(*session, window, cx);
                return;
            }
            Target::Restore { session } => {
                self.restore(*session, window, cx);
                return;
            }
        };
        let Some(session) = self.sessions.get(session_ix).cloned() else {
            return;
        };
        self.active = Some(session_ix);
        self.start = None;
        self._start_subscription = None;
        session.update(cx, |s, cx| match target {
            Target::Table { table, .. } if new_tab => s.open_table_in_new_tab(table, window, cx),
            Target::Table { table, .. } => s.open_table(table, None, window, cx),
            Target::Dashboard { dashboard, .. } => s.open_dashboard(dashboard, window, cx),
            Target::Query { sql, .. } => s.open_query(sql, window, cx),
            Target::Connect(_) | Target::Backup { .. } | Target::Restore { .. } => {}
        });
    }

    fn notify_user(&mut self, text: String, is_error: bool, cx: &mut Context<Self>) {
        self.notice = Some((text, is_error));
        cx.notify();
    }

    /// Asks where to save, then backs up the session's database there.
    fn backup(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(ix) else {
            return;
        };
        let (conn, name) = {
            let s = session.read(cx);
            (s.conn.clone(), s.name.to_string())
        };
        let ext = if conn.info().driver.as_str() == "sqlite" {
            "sqlite"
        } else {
            "sql"
        };
        let file = format!(
            "{name}-{}.{ext}",
            chrono::Local::now().format("%Y%m%d-%H%M")
        );
        let dir = dirs::document_dir().unwrap_or_else(std::env::temp_dir);
        let path = cx.prompt_for_new_path(&dir, Some(&file));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let _ = this.update(cx, |w, cx| {
                w.notify_user(format!("Backing up {name}…"), false, cx);
                let started = std::time::Instant::now();
                w._job = Some(crate::runtime::spawn_db(
                    cx,
                    async move {
                        match conn.backup() {
                            Some(b) => {
                                b.backup(&path, tokio_util::sync::CancellationToken::new())
                                    .await
                            }
                            None => Err(types::Error::unsupported("backup")),
                        }
                    },
                    move |w: &mut Self, result, cx| match result {
                        Ok(report) => w.notify_user(
                            format!(
                                "Backed up {name} to {} ({}, {:.1} s)",
                                report.path.display(),
                                ui::bytes(report.bytes),
                                started.elapsed().as_secs_f32()
                            ),
                            false,
                            cx,
                        ),
                        Err(e) => w.notify_user(format!("Backup of {name} failed: {e}"), true, cx),
                    },
                ));
            });
        })
        .detach();
    }

    /// Asks for a backup file, confirms, then restores it into the session's database.
    fn restore(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(ix).cloned() else {
            return;
        };
        let (conn, name) = {
            let s = session.read(cx);
            (s.conn.clone(), s.name.to_string())
        };
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Restore".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Ok(answer) = this.update_in(cx, |_, window, cx| {
                window.prompt(
                    gpui::PromptLevel::Critical,
                    &format!("Restore {file} into {name}?"),
                    Some("Objects in the backup are dropped and recreated. This cannot be undone."),
                    &["Restore", "Cancel"],
                    cx,
                )
            }) else {
                return;
            };
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |w, cx| {
                w.notify_user(format!("Restoring {file} into {name}…"), false, cx);
                w._job = Some(crate::runtime::spawn_db(
                    cx,
                    async move {
                        match conn.backup() {
                            Some(b) => {
                                b.restore(&path, tokio_util::sync::CancellationToken::new())
                                    .await
                            }
                            None => Err(types::Error::unsupported("restore")),
                        }
                    },
                    move |w: &mut Self, result, cx| match result {
                        Ok(()) => {
                            session.update(cx, |s, cx| s.refresh(cx));
                            w.notify_user(format!("Restored {file} into {name}"), false, cx);
                        }
                        Err(e) => {
                            w.notify_user(format!("Restore into {name} failed: {e}"), true, cx)
                        }
                    },
                ));
            });
        })
        .detach();
    }

    fn render_notice(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (text, is_error) = self.notice.clone()?;
        Some(
            div()
                .absolute()
                .bottom(px(16.))
                .right(px(16.))
                .max_w(px(520.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(8.))
                .bg(c(theme::BG))
                .border_1()
                .border_color(c(if is_error {
                    theme::DELETED_TEXT
                } else {
                    theme::LINE
                }))
                .shadow_lg()
                .text_size(px(13.))
                .text_color(c(if is_error {
                    theme::DELETED_TEXT
                } else {
                    theme::TEXT
                }))
                .child(div().flex_1().min_w_0().child(SharedString::from(text)))
                .child(
                    ui::icon_button("dismiss-notice", "close", 22., 13.).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.notice = None;
                            cx.notify();
                        },
                    )),
                ),
        )
    }

    /// Shows the connections list.
    fn show_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let screen = cx.new(|cx| ConnectionsScreen::new(window, cx));
        self._start_subscription = Some(cx.subscribe_in(
            &screen,
            window,
            |this, _, event: &ConnectionsEvent, window, cx| match event {
                ConnectionsEvent::Connect(saved) => this.connect(saved.clone(), window, cx),
                ConnectionsEvent::New => this.show_dialog(None, window, cx),
                ConnectionsEvent::Edit(saved) => this.show_dialog(Some(saved.clone()), window, cx),
            },
        ));
        self.start = Some(Start::List(screen));
        cx.notify();
    }

    fn show_dialog(
        &mut self,
        editing: Option<SavedConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let groups: Vec<String> = state::manager(cx)
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| s.group)
            .collect();
        let dialog = cx.new(|cx| ConnectionDialog::new(editing, groups, window, cx));
        self._start_subscription = Some(cx.subscribe_in(
            &dialog,
            window,
            |this, _, event: &DialogEvent, window, cx| match event {
                DialogEvent::Saved(connection) => {
                    let connection = (**connection).clone();
                    this.show_list(window, cx);
                    this.connect(connection, window, cx);
                }
                DialogEvent::Deleted | DialogEvent::Cancelled => this.show_list(window, cx),
            },
        ));
        self.start = Some(Start::Dialog(dialog));
        cx.notify();
    }

    fn set_status(&self, text: String, is_error: bool, cx: &mut Context<Self>) {
        if let Some(Start::List(screen)) = &self.start {
            screen.update(cx, |s, cx| {
                s.status = Some((text, is_error));
                cx.notify();
            });
        }
    }

    /// Connects on the I/O runtime, then opens a session tab.
    fn connect(&mut self, saved: SavedConnection, window: &mut Window, cx: &mut Context<Self>) {
        self.set_status(format!("Connecting to {}…", saved.name), false, cx);
        let manager = state::manager(cx);
        let (tx, rx) = futures::channel::oneshot::channel();
        let target = saved.clone();
        cx.global::<crate::runtime::Io>().0.spawn_task(async move {
            let password = manager.password(target.id);
            let result = match password {
                Ok(password) => {
                    manager
                        .connect(&conn::ConnectionManager::config(&target, password))
                        .await
                }
                Err(e) => Err(e),
            };
            let _ = tx.send(result);
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else { return };
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(conn) => {
                    // The connections search had focus and is going away; keep ⌘K working.
                    window.focus(&this.focus, cx);
                    let tag = saved.tag.unwrap_or(Tag::Green);
                    let session = cx.new(|cx| {
                        let mut s = Session::new(
                            saved.name.clone().into(),
                            tag,
                            saved.safe_mode,
                            saved.id,
                            conn,
                            window,
                            cx,
                        );
                        // Dev check: `RIFT_DEV_QUERY` opens a query tab with that SQL.
                        if let Ok(sql) = std::env::var("RIFT_DEV_QUERY") {
                            s.open_query(sql, window, cx);
                        }
                        // Dev check: `RIFT_DEV_TABLE=schema.table` opens that table.
                        if let Ok(path) = std::env::var("RIFT_DEV_TABLE") {
                            let (schema, name) = path
                                .split_once('.')
                                .map_or((None, path.as_str()), |(s, n)| (Some(s.into()), n));
                            s.open_table(types::TableRef::new(schema, name), None, window, cx);
                        }
                        s
                    });
                    this.sessions.push(session);
                    this.active = Some(this.sessions.len() - 1);
                    this.start = None;
                    this._start_subscription = None;
                    cx.notify();
                }
                Err(e) => this.set_status(format!("{}: {e}", saved.name), true, cx),
            });
        })
        .detach();
    }

    fn close_session(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.sessions.len() {
            let session = self.sessions.remove(ix);
            let conn = session.read(cx).conn.clone();
            crate::runtime::spawn_db(cx, async move { conn.close().await }, |_, _, _| {}).detach();
            self.active = match self.active {
                _ if self.sessions.is_empty() => None,
                Some(a) if a >= ix && a > 0 => Some(a - 1),
                other => other,
            };
            if self.sessions.is_empty() {
                self.show_list(window, cx);
            }
            cx.notify();
        }
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.start.is_some() {
            window.focus(&self.focus, cx);
        }
        self.active = Some(ix);
        self.start = None;
        self._start_subscription = None;
        self.switcher = false;
        self.switcher_menu = None;
        cx.notify();
    }

    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let brand = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(30.))
            .pr(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(8.))
                    .bg(c(theme::PRIMARY))
                    .child(ui::icon("database", 15., theme::BG)),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("rift"),
            );
        let session = self
            .start
            .is_none()
            .then(|| self.active.and_then(|ix| self.sessions.get(ix)))
            .flatten()
            .map(|s| {
                let s = s.read(cx);
                Crumbs {
                    name: s.name.clone(),
                    engine: s.engine(),
                    read_only: s.safe_mode == SafeMode::ReadOnly,
                    database: s.database(),
                    schema: s.schema().map(|s| s.to_string()).unwrap_or_default(),
                    pickable: s.schemas().len() > 1,
                    can_query: s.conn.query().is_some(),
                }
            });
        let can_query = session.as_ref().is_some_and(|s| s.can_query);
        let mut crumbs = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .min_w_0()
            .child(brand);
        if let Some(s) = session {
            let others = self.sessions.len() - 1;
            crumbs = crumbs.child(separator()).child(
                crumb("crumb-connection")
                    .relative()
                    .child(ui::engine_tile(&s.engine, 22.))
                    .child(s.name)
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(c(theme::FAINT))
                            .child(s.engine),
                    )
                    .when(s.read_only, |d| {
                        d.child(ui::badge(
                            "READ-ONLY",
                            theme::DELETED_TEXT,
                            theme::DELETED_BG,
                        ))
                    })
                    .when(others > 0, |d| {
                        d.child(
                            div()
                                .px(px(6.))
                                .rounded(px(6.))
                                .bg(c(theme::RAISED))
                                .text_size(px(11.))
                                .text_color(c(theme::MUTED))
                                .child(SharedString::from(format!("+{others}"))),
                        )
                    })
                    .child(ui::icon("chevron-down", 12., theme::FAINT))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.switcher = !this.switcher;
                        this.switcher_menu = None;
                        cx.notify();
                    }))
                    .children(self.switcher.then(|| self.render_switcher(cx))),
            );
            let (database, schema) = (s.database, s.schema);
            // Engines without a schema level (MySQL) report the database as the schema.
            if !database.is_empty() && database != schema {
                crumbs = crumbs
                    .child(separator())
                    .child(crumb("crumb-database").child(SharedString::from(database)));
            }
            if !schema.is_empty() {
                let pickable = s.pickable;
                crumbs = crumbs.child(separator()).child(
                    crumb("crumb-schema")
                        .relative()
                        .child(SharedString::from(schema))
                        .when(pickable, |d| {
                            d.child(ui::icon("chevron-down", 12., theme::FAINT))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.schema_menu = !this.schema_menu;
                                    cx.notify();
                                }))
                        })
                        .children(self.schema_menu.then(|| self.render_schema_menu(cx))),
                );
            }
        }
        let right = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .ml_auto()
            .child(
                div()
                    .id("open-anything")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(32.))
                    .w(px(240.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .text_size(px(13.))
                    .text_color(c(theme::FAINT))
                    .cursor_pointer()
                    .child(ui::icon("search", 14., theme::FAINT))
                    .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx)))
                    .child(div().flex_1().child("Search or jump to…"))
                    .child(
                        ui::kbd("⌘K")
                            .px(px(6.))
                            .rounded(px(6.))
                            .bg(c(theme::RAISED))
                            .text_color(c(theme::MUTED)),
                    ),
            )
            .when(can_query, |d| {
                d.child(
                    ui::secondary_button("sql", "SQL")
                        .child(ui::icon("terminal", 14., theme::TEXT))
                        .flex_row_reverse()
                        .on_click(cx.listener(|this, _, window, cx| {
                            let session = this.active.and_then(|ix| this.sessions.get(ix)).cloned();
                            if let Some(session) = session {
                                this.start = None;
                                this._start_subscription = None;
                                session.update(cx, |s, cx| s.open_query(String::new(), window, cx));
                            }
                        })),
                )
            });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .py(px(12.))
            .px(px(18.))
            .child(crumbs)
            .child(right)
    }

    /// The connection switcher dropdown: open connections, then connect / new.
    fn render_switcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.sessions.iter().enumerate().map(|(ix, session)| {
            let s = session.read(cx);
            let current = self.start.is_none() && self.active == Some(ix);
            let menu = self.switcher_menu == Some(ix);
            div()
                .id(("switch-conn", ix))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(56.))
                .flex_none()
                .px(px(10.))
                .rounded(px(10.))
                .cursor_pointer()
                .when(current, |d| d.bg(c(theme::RAISED)))
                .hover(|d| d.bg(c(theme::RAISED)))
                .child(ui::engine_tile(&s.engine(), 36.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(s.name.clone())
                                .when(s.safe_mode == SafeMode::ReadOnly, |d| {
                                    d.child(ui::badge(
                                        "READ-ONLY",
                                        theme::DELETED_TEXT,
                                        theme::DELETED_BG,
                                    ))
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(c(theme::FAINT))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(SharedString::from(format!(
                                    "{} · {}",
                                    s.engine(),
                                    s.conn.info().address
                                ))),
                        ),
                )
                .map(|d| {
                    if menu {
                        d.child(
                            ui::ghost_button(("disconnect", ix), "Disconnect")
                                .text_color(c(theme::DELETED_TEXT))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.switcher = false;
                                    this.switcher_menu = None;
                                    this.close_session(ix, window, cx);
                                })),
                        )
                    } else {
                        d.when(current, |d| d.child(ui::icon("check", 14., theme::TEXT)))
                    }
                })
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, _, _, cx| {
                        this.switcher_menu = Some(ix);
                        cx.notify();
                    }),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.activate(ix, window, cx);
                }))
        });
        let action = |id: &'static str, icon: &'static str, label: &'static str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(36.))
                .px(px(10.))
                .rounded(px(10.))
                .cursor_pointer()
                .text_size(px(13.))
                .hover(|d| d.bg(c(theme::RAISED)))
                .child(ui::icon(icon, 14., theme::MUTED))
                .child(div().flex_1().child(label))
        };
        gpui::deferred(
            gpui::anchored().snap_to_window().child(
                div()
                    .id("switcher")
                    .occlude()
                    .mt(px(36.))
                    .w(px(396.))
                    .p(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .rounded(px(14.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .shadow_lg()
                    .text_color(c(theme::TEXT))
                    .font_weight(FontWeight::NORMAL)
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.switcher = false;
                        this.switcher_menu = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .px(px(10.))
                            .pt(px(6.))
                            .pb(px(6.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(theme::FAINT))
                            .child(SharedString::from(format!(
                                "OPEN CONNECTIONS · {}",
                                self.sessions.len()
                            ))),
                    )
                    .children(rows)
                    .child(div().h(px(1.)).my(px(6.)).mx(px(2.)).bg(c(theme::LINE)))
                    .child(
                        action("switch-saved", "search", "Connect to a saved connection…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.switcher = false;
                                this.show_list(window, cx);
                            })),
                    )
                    .child(
                        action("switch-new", "plus", "New connection").on_click(cx.listener(
                            |this, _, window, cx| {
                                cx.stop_propagation();
                                this.switcher = false;
                                this.show_dialog(None, window, cx);
                            },
                        )),
                    ),
            ),
        )
        .with_priority(1)
    }

    /// The schema crumb dropdown.
    fn render_schema_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.active.and_then(|ix| self.sessions.get(ix)).cloned();
        let (schemas, current) = session.as_ref().map_or((Vec::new(), None), |s| {
            let s = s.read(cx);
            (s.schemas().to_vec(), s.schema().cloned())
        });
        gpui::deferred(
            gpui::anchored().snap_to_window().child(
                div()
                    .id("schema-menu")
                    .occlude()
                    .mt(px(36.))
                    .w(px(224.))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .p(px(8.))
                    .rounded(px(12.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .shadow_lg()
                    .font_weight(FontWeight::NORMAL)
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.schema_menu = false;
                        cx.notify();
                    }))
                    .children(schemas.into_iter().enumerate().map(|(i, schema)| {
                        let is_current = current.as_ref() == Some(&schema);
                        let session = session.clone();
                        div()
                            .id(("schema-option", i))
                            .flex()
                            .items_center()
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .when(is_current, |d| d.bg(c(theme::RAISED)))
                            .hover(|d| d.bg(c(theme::RAISED)))
                            .child(SharedString::from(schema.to_string()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.schema_menu = false;
                                if let Some(session) = &session {
                                    let schema = schema.clone();
                                    session.update(cx, |s, cx| s.set_schema(schema, cx));
                                }
                                cx.notify();
                            }))
                    })),
            ),
        )
        .with_priority(1)
    }
}

/// What the breadcrumb shows for the active session.
struct Crumbs {
    name: SharedString,
    engine: SharedString,
    read_only: bool,
    database: String,
    schema: String,
    pickable: bool,
    can_query: bool,
}

/// The `/` between breadcrumb segments.
fn separator() -> impl IntoElement {
    div()
        .px(px(2.))
        .text_size(px(18.))
        .font_weight(FontWeight::LIGHT)
        .text_color(c(0xd4d4d8))
        .child("/")
}

/// A breadcrumb segment button.
fn crumb(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.))
        .h(px(30.))
        .px(px(8.))
        .rounded(px(8.))
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .hover(|d| d.bg(c(theme::RAISED)))
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("Workspace::render").entered();
        let started = Instant::now();
        let body: AnyElement = match (
            &self.start,
            self.active.and_then(|ix| self.sessions.get(ix)),
        ) {
            (Some(Start::List(screen)), _) => screen.clone().into_any_element(),
            (Some(Start::Dialog(dialog)), _) => dialog.clone().into_any_element(),
            (None, Some(session)) => session.clone().into_any_element(),
            (None, None) => div()
                .relative()
                .size_full()
                .child(ui::dot_grid())
                .into_any_element(),
        };
        let overlay = self.palette.as_ref().map(|(palette, _)| {
            let mut veil = theme::hsla(theme::BG);
            veil.a = 0.82;
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(veil)
                .child(ui::dot_grid())
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .child(palette.clone())
        });
        div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleSwitcher, _, cx| {
                if !this.sessions.is_empty() {
                    this.switcher = !this.switcher;
                    cx.notify();
                }
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(c(theme::PANEL))
            .font_family(theme::SANS)
            .text_size(px(13.))
            .text_color(c(theme::TEXT))
            .when(!self.sessions.is_empty(), |d| {
                d.child(self.render_top_bar(cx))
            })
            .child(div().flex_1().min_h_0().child(body))
            .children(self.render_notice(cx))
            .children(overlay)
            .child(
                canvas(
                    move |_, _, _| {
                        let ms = started.elapsed().as_millis();
                        if ms > FRAME_BUDGET_MS && (cfg!(debug_assertions) || frame_log_enabled()) {
                            tracing::warn!(ms, "slow frame");
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_0(),
            )
    }
}
