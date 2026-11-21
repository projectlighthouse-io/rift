//! Range browser for ordered key-value stores with a directory layer (FoundationDB): the
//! directory tree sidebar and one range-scan document per directory. Shown when a connection
//! has `key_directories()`; keys decode through `key_encoding()` when it has one.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    Context, Entity, EventEmitter, FontWeight, SharedString, Subscription, Task, Window, div, px,
};
use gpui_component::input::{Editor, EditorState, InputEvent, InputState};
use traits::Connection;
use types::{
    Column, Error, KeyBytes, KeyDirectory, KeyEntry, KeyMatch, KeyScan, KvValue, Row, ScanCursor,
    Value,
};

use crate::dropdown::Dropdown;
use crate::grid::{Grid, GridEvent};
use crate::theme::{self, c};
use crate::{runtime, ui};

const DEFAULT_LIMIT: &str = "100";
/// Directory counts stop here and show `1,000,000+`.
const COUNT_CAP: u64 = 1_000_000;
const PANEL_WIDTH: f32 = 290.;

/// `\x15\x02user\x00`: printable ASCII as is, everything else as `\xNN`.
fn printable(bytes: &[u8]) -> String {
    let mut out = String::new();
    for b in bytes {
        if (b.is_ascii_graphic() && *b != b'\\') || *b == b' ' {
            out.push(char::from(*b));
        } else {
            let _ = write!(out, "\\x{b:02x}");
        }
    }
    out
}

/// Inverse of [`printable`].
fn unprintable(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'x') {
            let hex = text.get(i + 2..i + 4).ok_or("short \\x escape")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| format!("bad escape \\x{hex}"))?);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(out)
}

/// The first key after every key starting with `prefix`.
fn strinc(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last != 0xff {
            end.push(last + 1);
            return end;
        }
    }
    vec![0xff]
}

fn bytes_label(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else {
        ui::bytes(n as u64)
    }
}

/// Asks the session to open a directory's range document.
pub(crate) struct OpenDirectory(pub(crate) KeyDirectory);

/// The "Show system keys" toggle changed.
pub(crate) struct ShowSystem(pub(crate) bool);

/// Sidebar: the directory layer as a tree, with key counts.
pub(crate) struct DirectoryTree {
    conn: Arc<dyn Connection>,
    name: SharedString,
    dirs: Vec<KeyDirectory>,
    counts: HashMap<Vec<String>, u64>,
    collapsed: HashSet<Vec<String>>,
    selected: Option<Vec<String>>,
    filter: Entity<InputState>,
    show_system: bool,
    error: Option<String>,
    _task: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<OpenDirectory> for DirectoryTree {}
impl EventEmitter<ShowSystem> for DirectoryTree {}

impl DirectoryTree {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter directories"));
        let subscription =
            cx.subscribe_in(&filter, window, |_, _, _: &InputEvent, _, cx| cx.notify());
        let mut tree = Self {
            conn,
            name,
            dirs: Vec::new(),
            counts: HashMap::new(),
            collapsed: HashSet::new(),
            selected: None,
            filter,
            show_system: false,
            error: None,
            _task: None,
            _subscription: subscription,
        };
        tree.reload(cx);
        tree
    }

    /// Lists directories, then counts each one's keys in the background.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let io: Arc<dyn traits::Spawner> = cx.global::<runtime::Io>().0.clone();
        self._task = Some(cx.spawn(async move |this, cx| {
            let dirs = traits::spawner::run(&*io, {
                let conn = conn.clone();
                async move {
                    match conn.key_directories() {
                        Some(d) => d.directories().await,
                        None => Err(Error::unsupported("directories")),
                    }
                }
            })
            .await
            .and_then(|r| r);
            let dirs = match dirs {
                Ok(dirs) => dirs,
                Err(e) => {
                    let _ = this.update(cx, |t, cx| {
                        t.error = Some(e.to_string());
                        cx.notify();
                    });
                    return;
                }
            };
            let _ = this.update(cx, |t, cx| {
                t.dirs = dirs.clone();
                t.error = None;
                cx.notify();
            });
            for dir in dirs {
                let conn = conn.clone();
                let prefix = dir.prefix.clone();
                let count = traits::spawner::run(&*io, async move {
                    match conn.key_directories() {
                        Some(d) => d.count(&prefix, COUNT_CAP).await,
                        None => Ok(0),
                    }
                })
                .await
                .and_then(|r| r);
                if let Ok(n) = count
                    && this
                        .update(cx, |t, cx| {
                            t.counts.insert(dir.path.clone(), n);
                            cx.notify();
                        })
                        .is_err()
                {
                    return;
                }
            }
        }));
    }

    fn has_children(&self, path: &[String]) -> bool {
        self.dirs
            .iter()
            .any(|d| d.path.len() > path.len() && d.path.starts_with(path))
    }

    fn visible(&self, dir: &KeyDirectory, filter: &str) -> bool {
        if !filter.is_empty() {
            return dir.path.join("/").to_lowercase().contains(filter);
        }
        (1..dir.path.len()).all(|n| !self.collapsed.contains(&dir.path[..n]))
    }
}

impl Render for DirectoryTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let filter = self.filter.read(cx).value().to_lowercase();
        let rows: Vec<_> = self
            .dirs
            .iter()
            .filter(|d| self.visible(d, &filter))
            .enumerate()
            .map(|(i, dir)| {
                let path = dir.path.clone();
                let depth = if filter.is_empty() {
                    dir.path.len() - 1
                } else {
                    0
                };
                let open = !self.collapsed.contains(&path);
                let selected = self.selected.as_ref() == Some(&path);
                let label = if filter.is_empty() {
                    dir.path.last().cloned().unwrap_or_default()
                } else {
                    dir.path.join("/")
                };
                let toggle_path = path.clone();
                let open_dir = dir.clone();
                div()
                    .id(("dir", i))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .pl(px(8. + depth as f32 * 18.))
                    .pr(px(10.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_color(c(if selected { theme::TEXT } else { theme::MUTED }))
                    .when(selected, |d| d.bg(c(theme::SELECTION)))
                    .hover(|d| d.bg(c(theme::SELECTION)))
                    .child(div().id(("dir-toggle", i)).w(px(14.)).when(
                        self.has_children(&path),
                        |d| {
                            d.child(ui::icon(
                                if open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                13.,
                                theme::FAINT,
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if !this.collapsed.remove(&toggle_path) {
                                        this.collapsed.insert(toggle_path.clone());
                                    }
                                    cx.notify();
                                },
                            ))
                        },
                    ))
                    .child(ui::icon(
                        "folder",
                        14.,
                        if selected {
                            theme::ACCENT_TEXT
                        } else {
                            theme::FAINT
                        },
                    ))
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_size(px(12.5))
                            .child(SharedString::from(label)),
                    )
                    // Parents usually hold no keys of their own; a 0 there is noise.
                    .children(
                        self.counts
                            .get(&path)
                            .filter(|n| **n > 0 || !self.has_children(&path))
                            .map(|n| {
                                div()
                                    .ml_auto()
                                    .text_size(px(11.5))
                                    .text_color(c(theme::FAINT))
                                    .child(SharedString::from(if *n >= COUNT_CAP {
                                        format!("{}+", ui::thousands(*n))
                                    } else {
                                        ui::thousands(*n)
                                    }))
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(open_dir.path.clone());
                        cx.emit(OpenDirectory(open_dir.clone()));
                        cx.notify();
                    }))
            })
            .collect();
        let empty = self.dirs.is_empty() && self.error.is_none();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(250.))
            .h_full()
            .bg(c(theme::PANEL))
            .border_r_1()
            .border_color(c(theme::LINE))
            .child(
                div()
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
                            .child("CLUSTER"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(px(6.))
                            .bg(c(theme::RAISED))
                            .border_1()
                            .border_color(c(theme::LINE))
                            .child(ui::icon("database", 14., theme::ACCENT_TEXT))
                            .child(
                                div()
                                    .font_family(mono.clone())
                                    .text_size(px(12.5))
                                    .child(self.name.clone()),
                            )
                            .child(
                                div()
                                    .id("reload-dirs")
                                    .ml_auto()
                                    .cursor_pointer()
                                    .child(ui::icon("refresh", 13., theme::FAINT))
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(30.))
                            .pl(px(9.))
                            .pr(px(4.))
                            .rounded(px(6.))
                            .bg(c(theme::BG))
                            .border_1()
                            .border_color(c(theme::LINE))
                            .child(ui::icon("search", 13., theme::FAINT))
                            .child(div().flex_1().child(ui::input(&self.filter, 13.))),
                    ),
            )
            .child(
                div()
                    .id("directories")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap(px(2.))
                    .px(px(6.))
                    .py(px(8.))
                    .children(rows)
                    .children(self.error.clone().map(|e| {
                        div()
                            .p(px(8.))
                            .text_size(px(12.5))
                            .text_color(c(theme::DELETED_TEXT))
                            .child(SharedString::from(e))
                    }))
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(8.))
                                .text_size(px(12.5))
                                .text_color(c(theme::FAINT))
                                .child("No directories"),
                        )
                    })
                    .child(
                        div()
                            .id("show-system")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .mt(px(8.))
                            .px(px(8.))
                            .h(px(28.))
                            .cursor_pointer()
                            .text_size(px(12.5))
                            .text_color(c(theme::MUTED))
                            .child(checkbox(self.show_system))
                            .child("Show system keys (\\xff)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_system = !this.show_system;
                                cx.emit(ShowSystem(this.show_system));
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(c(theme::LINE))
                    .text_size(px(12.))
                    .text_color(c(theme::FAINT))
                    .child(if self.conn.key_encoding().is_some() {
                        "Directory layer · tuple keys decoded"
                    } else {
                        "Directory layer"
                    }),
            )
    }
}

fn checkbox(on: bool) -> impl IntoElement {
    div()
        .size(px(15.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .bg(c(if on { theme::PRIMARY } else { theme::BG }))
        .border_1()
        .border_color(c(if on { theme::PRIMARY } else { theme::LINE }))
        .children(on.then(|| ui::icon("check", 11., theme::BG)))
}

/// How keys are shown and typed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Decode {
    Encoding,
    Raw,
    Utf8,
}

/// Right panel mode.
enum Panel {
    Value,
    Editing(Entity<EditorState>),
    Setting {
        key: Entity<InputState>,
        value: Entity<EditorState>,
    },
}

/// One directory's range scan.
pub(crate) struct RangeView {
    conn: Arc<dyn Connection>,
    pub(crate) dir: KeyDirectory,
    include_system: bool,
    begin: Entity<InputState>,
    end: Entity<InputState>,
    limit: Entity<InputState>,
    decode: Entity<Dropdown>,
    modes: Vec<Decode>,
    reverse: bool,
    grid: Entity<Grid>,
    entries: Vec<KeyEntry>,
    cursor: Option<ScanCursor>,
    read_version: Option<u64>,
    elapsed_ms: Option<u128>,
    error: Option<String>,
    selected: Option<usize>,
    panel: Panel,
    confirm_clear: bool,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RangeView {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        dir: KeyDirectory,
        include_system: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut modes = Vec::new();
        let mut labels: Vec<SharedString> = Vec::new();
        if let Some(enc) = conn.key_encoding() {
            modes.push(Decode::Encoding);
            labels.push(enc.name().into());
        }
        modes.extend([Decode::Raw, Decode::Utf8]);
        labels.extend(["Raw".into(), "UTF-8".into()]);
        let decode = cx.new(|_| Dropdown::new(labels, 0).height(30.));
        let begin = cx.new(|cx| InputState::new(window, cx).placeholder("start of directory"));
        let end = cx.new(|cx| InputState::new(window, cx).placeholder("end of directory"));
        let limit = cx.new(|cx| InputState::new(window, cx).default_value(DEFAULT_LIMIT));
        let grid = cx.new(|_| Grid::new());
        let mut subscriptions = vec![
            cx.subscribe_in(&grid, window, |this, _, e: &GridEvent, _, cx| {
                if let GridEvent::Selected { row } = e {
                    this.selected = Some(*row);
                    this.panel = Panel::Value;
                    this.confirm_clear = false;
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &decode,
                window,
                |this, _, _: &crate::dropdown::Changed, _, cx| this.fill_grid(cx),
            ),
        ];
        for input in [&begin, &end, &limit] {
            subscriptions.push(
                cx.subscribe_in(input, window, |this, _, e: &InputEvent, _, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        this.scan(false, cx);
                    }
                }),
            );
        }
        let mut view = Self {
            conn,
            dir,
            include_system,
            begin,
            end,
            limit,
            decode,
            modes,
            reverse: false,
            grid,
            entries: Vec::new(),
            cursor: None,
            read_version: None,
            elapsed_ms: None,
            error: None,
            selected: None,
            panel: Panel::Value,
            confirm_clear: false,
            _task: None,
            _subscriptions: subscriptions,
        };
        view.scan(false, cx);
        view
    }

    pub(crate) fn set_include_system(&mut self, on: bool, cx: &mut Context<Self>) {
        self.include_system = on;
        self.scan(false, cx);
    }

    fn mode(&self, cx: &gpui::App) -> Decode {
        self.modes[self.decode.read(cx).selected().min(self.modes.len() - 1)]
    }

    /// Key text relative to the directory, in the chosen mode.
    fn key_text(&self, key: &[u8], cx: &gpui::App) -> String {
        let prefix = self.dir.prefix.as_bytes();
        let relative = key.strip_prefix(prefix).unwrap_or(key);
        match self.mode(cx) {
            Decode::Encoding => self
                .conn
                .key_encoding()
                .and_then(|e| e.decode(relative))
                .unwrap_or_else(|| printable(relative)),
            Decode::Raw => printable(key),
            Decode::Utf8 => String::from_utf8_lossy(relative).into_owned(),
        }
    }

    /// Bytes for a typed bound or key, relative to the directory.
    fn parse_text(&self, text: &str, cx: &gpui::App) -> Result<Vec<u8>, String> {
        match self.mode(cx) {
            Decode::Encoding => match self.conn.key_encoding() {
                Some(e) => e.encode(text).map_err(|e| e.to_string()),
                None => Err("no key encoding".into()),
            },
            Decode::Raw => unprintable(text),
            Decode::Utf8 => Ok(text.as_bytes().to_vec()),
        }
    }

    fn bounds(&self, cx: &gpui::App) -> Result<(Vec<u8>, Vec<u8>), String> {
        let prefix = self.dir.prefix.as_bytes().to_vec();
        let with_prefix = |text: &str| -> Result<Option<Vec<u8>>, String> {
            let text = text.trim();
            if text.is_empty() {
                return Ok(None);
            }
            let mut key = prefix.clone();
            key.extend(self.parse_text(text, cx)?);
            Ok(Some(key))
        };
        let begin = with_prefix(&self.begin.read(cx).value())?.unwrap_or_else(|| prefix.clone());
        let end = with_prefix(&self.end.read(cx).value())?.unwrap_or_else(|| strinc(&prefix));
        Ok((begin, end))
    }

    /// Scans the range; `next` continues from the last page.
    fn scan(&mut self, next: bool, cx: &mut Context<Self>) {
        let (begin, end) = match self.bounds(cx) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let limit = self
            .limit
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .unwrap_or(100)
            .clamp(1, 10_000);
        let scan = KeyScan {
            matching: KeyMatch::Range {
                begin: KeyBytes::new(begin),
                end: KeyBytes::new(end),
            },
            limit,
            reverse: self.reverse,
            cursor: if next { self.cursor.clone() } else { None },
            include_system: self.include_system,
        };
        let conn = self.conn.clone();
        let started = Instant::now();
        self._task = Some(runtime::spawn_db(
            cx,
            async move {
                match conn.key_value() {
                    Some(kv) => kv.scan(&scan).await,
                    None => Err(Error::unsupported("key scans")),
                }
            },
            move |this: &mut Self, result, cx| {
                this.elapsed_ms = Some(started.elapsed().as_millis());
                match result {
                    Ok(page) => {
                        this.entries = page.entries;
                        this.cursor = page.cursor;
                        this.read_version = page.read_version;
                        this.error = None;
                        this.selected = None;
                        this.panel = Panel::Value;
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                this.fill_grid(cx);
            },
        ));
    }

    fn fill_grid(&mut self, cx: &mut Context<Self>) {
        let key_header = match self.mode(cx) {
            Decode::Encoding => format!(
                "Key ({})",
                self.conn
                    .key_encoding()
                    .map_or("decoded", |e| e.name())
                    .to_lowercase()
            ),
            Decode::Raw => "Key (raw)".into(),
            Decode::Utf8 => "Key (UTF-8)".into(),
        };
        let columns = vec![
            Column::new(key_header.as_str(), ""),
            Column::new("Raw key", ""),
            Column::new("Value", ""),
            Column::new("Size", ""),
        ];
        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|e| {
                let value = e.value.as_deref().unwrap_or_default();
                let mut raw = printable(e.key.as_bytes());
                if raw.chars().count() > 30 {
                    raw = format!("{}…", raw.chars().take(29).collect::<String>());
                }
                let mut shown = String::from_utf8_lossy(value).replace('\n', " ");
                if shown.chars().count() > 80 {
                    shown = format!("{}…", shown.chars().take(79).collect::<String>());
                }
                Row {
                    values: vec![
                        Value::Text(self.key_text(e.key.as_bytes(), cx)),
                        Value::Text(raw),
                        Value::Text(shown),
                        Value::Text(bytes_label(value.len())),
                    ],
                }
            })
            .collect();
        self.grid.update(cx, |g, cx| {
            g.set_data(columns, rows, 1);
            cx.notify();
        });
        cx.notify();
    }

    fn write(&mut self, key: Vec<u8>, value: Option<Vec<u8>>, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        self._task = Some(runtime::spawn_db(
            cx,
            async move {
                let kv = conn
                    .key_value()
                    .ok_or_else(|| Error::unsupported("key writes"))?;
                let key = KeyBytes::new(key);
                match value {
                    Some(v) => kv.set(&key, &KvValue::String(v)).await,
                    None => kv.delete(&key).await.map(|_| ()),
                }
            },
            |this: &mut Self, result, cx| match result {
                Ok(()) => this.scan(false, cx),
                Err(e) => {
                    this.error = Some(e.to_string());
                    cx.notify();
                }
            },
        ));
    }

    fn start_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected.and_then(|i| self.entries.get(i)) else {
            return;
        };
        let text = String::from_utf8_lossy(entry.value.as_deref().unwrap_or_default()).into_owned();
        let editor = cx.new(|cx| EditorState::new(window, cx).default_value(pretty(&text)));
        self.panel = Panel::Editing(editor);
        cx.notify();
    }

    fn start_set(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = if self.mode(cx) == Decode::Encoding {
            "(\"user\", 1)"
        } else {
            "key"
        };
        let key = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let value = cx.new(|cx| EditorState::new(window, cx));
        key.update(cx, |s, cx| s.focus(window, cx));
        self.panel = Panel::Setting { key, value };
        cx.notify();
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let label = |t: &'static str| {
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(theme::FAINT))
                .child(t)
        };
        let boxed = |child: gpui::AnyElement| {
            div()
                .h(px(220.))
                .p(px(10.))
                .rounded(px(6.))
                .bg(c(theme::BG))
                .border_1()
                .border_color(c(theme::LINE))
                .font_family(mono.clone())
                .text_size(px(12.5))
                .child(child)
        };
        let body = match &self.panel {
            Panel::Setting { key, value } => {
                let (key, value) = (key.clone(), value.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(label("SET KEY"))
                    .child(
                        div()
                            .h(px(30.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .bg(c(theme::BG))
                            .border_1()
                            .border_color(c(theme::LINE))
                            .font_family(mono.clone())
                            .child(ui::input(&key, 12.5)),
                    )
                    .child(boxed(
                        Editor::new(&value)
                            .appearance(false)
                            .h_full()
                            .into_any_element(),
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(ui::primary_button("set-save", "Save").on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let text = key.read(cx).value().to_string();
                                    match this.parse_text(&text, cx) {
                                        Ok(rel) => {
                                            let mut full = this.dir.prefix.as_bytes().to_vec();
                                            full.extend(rel);
                                            let bytes =
                                                value.read(cx).value().to_string().into_bytes();
                                            this.panel = Panel::Value;
                                            this.write(full, Some(bytes), cx);
                                        }
                                        Err(e) => {
                                            this.error = Some(e);
                                            cx.notify();
                                        }
                                    }
                                },
                            )))
                            .child(
                                ui::ghost_button("set-cancel", "Cancel").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.panel = Panel::Value;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            }
            _ => match self.selected.and_then(|i| self.entries.get(i)) {
                None => div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(label("VALUE"))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(c(theme::FAINT))
                            .child("Select a key"),
                    )
                    .into_any_element(),
                Some(entry) => {
                    let value = entry.value.clone().unwrap_or_default();
                    let text = String::from_utf8_lossy(&value).into_owned();
                    let json = serde_json::from_str::<serde_json::Value>(&text).is_ok();
                    let utf8 = std::str::from_utf8(&value).is_ok();
                    let key = entry.key.as_bytes().to_vec();
                    let editing = match &self.panel {
                        Panel::Editing(e) => Some(e.clone()),
                        _ => None,
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(label("VALUE"))
                        .child(
                            div()
                                .font_family(mono.clone())
                                .text_size(px(12.5))
                                .child(SharedString::from(self.key_text(&key, cx))),
                        )
                        .child(match &editing {
                            Some(editor) => boxed(
                                Editor::new(editor)
                                    .appearance(false)
                                    .h_full()
                                    .into_any_element(),
                            ),
                            None => boxed(
                                div()
                                    .id("value-text")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .child(SharedString::from(if utf8 {
                                        pretty(&text)
                                    } else {
                                        printable(&value)
                                    }))
                                    .into_any_element(),
                            ),
                        })
                        .child(div().text_size(px(12.)).text_color(c(theme::MUTED)).child(
                            SharedString::from(format!(
                                "{} · {}",
                                bytes_label(value.len()),
                                if json {
                                    "JSON"
                                } else if utf8 {
                                    "Text"
                                } else {
                                    "Binary"
                                }
                            )),
                        ))
                        .child(match editing {
                            Some(editor) => {
                                let key = key.clone();
                                let original = text.clone();
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(ui::primary_button("value-save", "Save").on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            let edited = editor.read(cx).value().to_string();
                                            this.panel = Panel::Value;
                                            if let Some(bytes) = saved_bytes(&original, &edited) {
                                                this.write(key.clone(), Some(bytes), cx);
                                            } else {
                                                cx.notify();
                                            }
                                        }),
                                    ))
                                    .child(ui::ghost_button("value-cancel", "Cancel").on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.panel = Panel::Value;
                                            cx.notify();
                                        }),
                                    ))
                                    .into_any_element()
                            }
                            None => div()
                                .flex()
                                .gap(px(8.))
                                .child(ui::secondary_button("edit-value", "Edit value").on_click(
                                    cx.listener(|this, _, window, cx| this.start_edit(window, cx)),
                                ))
                                .child(
                                    ui::secondary_button(
                                        "clear-key",
                                        if self.confirm_clear {
                                            "Clear? Click again"
                                        } else {
                                            "Clear key"
                                        },
                                    )
                                    .text_color(c(theme::DELETED_TEXT))
                                    .border_color(c(theme::DELETED_BG))
                                    .flex_row_reverse()
                                    .child(ui::icon("trash", 13., theme::DELETED_TEXT))
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if this.confirm_clear {
                                                this.confirm_clear = false;
                                                this.write(key.clone(), None, cx);
                                            } else {
                                                this.confirm_clear = true;
                                                cx.notify();
                                            }
                                        },
                                    )),
                                )
                                .into_any_element(),
                        })
                        .into_any_element()
                }
            },
        };
        div()
            .flex_none()
            .w(px(PANEL_WIDTH))
            .h_full()
            .p(px(14.))
            .bg(c(theme::PANEL))
            .border_l_1()
            .border_color(c(theme::LINE))
            .child(body)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = theme::mono(cx);
        let field = |label: &'static str, w: f32, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(c(theme::MUTED))
                        .child(label),
                )
                .child(
                    div()
                        .w(px(w))
                        .h(px(30.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .bg(c(theme::BG))
                        .border_1()
                        .border_color(c(theme::LINE))
                        .font_family(mono.clone())
                        .child(ui::input(input, 12.5)),
                )
        };
        div()
            .flex()
            .flex_none()
            .items_end()
            .gap(px(10.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(c(theme::LINE))
            .child(field("Begin", 180., &self.begin))
            .child(field("End", 180., &self.end))
            .child(field("Limit", 72., &self.limit))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(theme::MUTED))
                            .child("Decode as"),
                    )
                    .child(div().w(px(110.)).child(self.decode.clone())),
            )
            .child(
                div()
                    .id("reverse")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(30.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(c(theme::MUTED))
                    .child(checkbox(self.reverse))
                    .child("Reverse")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reverse = !this.reverse;
                        this.scan(false, cx);
                    })),
            )
            .child(
                ui::primary_button("scan-range", "Scan range")
                    .flex_row_reverse()
                    .child(ui::icon("play", 12., theme::BG))
                    .on_click(cx.listener(|this, _, _, cx| this.scan(false, cx))),
            )
            .child(
                div().ml_auto().child(
                    ui::secondary_button("set-key", "Set key")
                        .flex_row_reverse()
                        .child(ui::icon("plus", 13., theme::TEXT))
                        .on_click(cx.listener(|this, _, window, cx| this.start_set(window, cx))),
                ),
            )
    }
}

/// Bytes to write for an edited value, or `None` when nothing changed. The editor shows
/// JSON pretty-printed; compact JSON stays compact when saved.
fn saved_bytes(original: &str, edited: &str) -> Option<Vec<u8>> {
    if edited == pretty(original) || edited == original {
        return None;
    }
    let compact =
        !original.contains('\n') && serde_json::from_str::<serde_json::Value>(original).is_ok();
    match serde_json::from_str::<serde_json::Value>(edited) {
        Ok(v) if compact => serde_json::to_string(&v).ok().map(String::into_bytes),
        _ => Some(edited.as_bytes().to_vec()),
    }
}

/// Pretty-prints JSON; other text is returned as is.
fn pretty(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| text.to_owned())
}

impl Render for RangeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sep = || div().text_color(c(theme::FAINT)).child("·");
        let status = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .h(px(34.))
            .border_t_1()
            .border_color(c(theme::LINE))
            .bg(c(theme::PANEL))
            .text_size(px(12.))
            .text_color(c(theme::MUTED))
            .child(match &self.error {
                Some(e) => div()
                    .text_color(c(theme::DELETED_TEXT))
                    .child(SharedString::from(e.clone()))
                    .into_any_element(),
                None => div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(SharedString::from(format!(
                        "{} keys",
                        ui::thousands(self.entries.len() as u64)
                    )))
                    .child(sep())
                    .child(SharedString::from(format!(
                        "{} ms",
                        self.elapsed_ms.unwrap_or(0)
                    )))
                    .children(self.read_version.map(|v| {
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(sep())
                            .child(SharedString::from(format!(
                                "read version {} · snapshot read",
                                ui::thousands(v)
                            )))
                    }))
                    .children(self.cursor.is_some().then(|| {
                        div()
                            .id("next-page")
                            .cursor_pointer()
                            .text_color(c(theme::ACCENT_TEXT))
                            .child("Next page →")
                            .on_click(cx.listener(|this, _, _, cx| this.scan(true, cx)))
                    }))
                    .into_any_element(),
            })
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(ui::icon("info", 12., theme::FAINT))
                    .child("Writes run in one transaction (5 s, 10 MB limit)"),
            );
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(c(theme::BG))
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(self.grid.clone()))
                    .child(self.render_panel(cx)),
            )
            .child(status)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn printable_round_trips() {
        let bytes = b"\x15\x02user\x00\\";
        assert_eq!(super::printable(bytes), "\\x15\\x02user\\x00\\x5c");
        assert_eq!(
            super::unprintable(&super::printable(bytes)).expect("parse"),
            bytes
        );
        assert_eq!(super::strinc(b"\x15\x02"), b"\x15\x03");
        let original = r#"{"name":"chloe","plan":"free"}"#;
        assert_eq!(super::saved_bytes(original, &super::pretty(original)), None);
        assert_eq!(
            super::saved_bytes(
                original,
                "{\n  \"name\": \"chloe\",\n  \"plan\": \"pro\"\n}"
            ),
            Some(br#"{"name":"chloe","plan":"pro"}"#.to_vec())
        );
    }
}
