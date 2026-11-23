//! Key-value screens, shown when a connection has `key_value()`: a key tree grouped by `:`,
//! one document per key with a per-type editor, and memory charts when `memory_stats()` is
//! available. Edits write straight to the store.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, SharedString, Subscription, Task,
    Window, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use traits::Connection;
use types::{
    Error, HashField, KeyBytes, KeyEntry, KeyKind, KeyMatch, KeyScan, KvValue, MemoryReport,
    ScoredMember,
};

use crate::theme::{self, c};
use crate::{runtime, ui};

/// Keys per `SCAN` page.
const SCAN_PAGE: u32 = 1_000;
/// Keys shown under an expanded group before "… N more".
const SHOWN_STEP: usize = 200;
const FIRST_SHOWN: usize = 4;

/// Asks the session to open a key.
pub(crate) struct OpenKey(pub(crate) KeyBytes);

/// Sidebar: database label, pattern filter and the key tree.
pub(crate) struct KeyTree {
    conn: Arc<dyn Connection>,
    pattern: Entity<InputState>,
    groups: BTreeMap<String, Vec<KeyEntry>>,
    seen: HashSet<KeyBytes>,
    expanded: HashSet<String>,
    shown: BTreeMap<String, usize>,
    total: Option<u64>,
    selected: Option<KeyBytes>,
    scanning: bool,
    error: Option<String>,
    _scan: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<OpenKey> for KeyTree {}

/// Group of a key: text up to and including the first `:`, or empty for top-level keys.
fn group_of(key: &KeyBytes) -> String {
    let text = key.to_display();
    text.split_once(':')
        .map(|(p, _)| format!("{p}:"))
        .unwrap_or_default()
}

pub(crate) fn kind_badge(kind: Option<KeyKind>) -> AnyElement {
    match kind {
        Some(k) => ui::badge(k.label(), theme::ACCENT_TEXT, theme::SELECTION).into_any_element(),
        None => div().into_any_element(),
    }
}

impl KeyTree {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pattern = cx.new(|cx| InputState::new(window, cx).placeholder("Pattern, e.g. cart:*"));
        let subscription =
            cx.subscribe_in(&pattern, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.rescan(cx);
                }
            });
        let mut tree = Self {
            conn,
            pattern,
            groups: BTreeMap::new(),
            seen: HashSet::new(),
            expanded: HashSet::new(),
            shown: BTreeMap::new(),
            total: None,
            selected: None,
            scanning: false,
            error: None,
            _scan: None,
            _subscription: subscription,
        };
        tree.rescan(cx);
        tree
    }

    /// Scans the whole keyspace in the background, one `SCAN` page at a time.
    pub(crate) fn rescan(&mut self, cx: &mut Context<Self>) {
        self.groups.clear();
        self.seen.clear();
        self.scanning = true;
        self.error = None;
        let text = self.pattern.read(cx).value().trim().to_owned();
        let matching = if text.is_empty() {
            KeyMatch::All
        } else {
            KeyMatch::Pattern(text)
        };
        let conn = self.conn.clone();
        let (tx, mut rx) = mpsc::unbounded::<Result<Vec<KeyEntry>, Error>>();
        cx.global::<runtime::Io>().0.spawn_task(async move {
            let Some(kv) = conn.key_value() else { return };
            let mut scan = KeyScan {
                matching,
                limit: SCAN_PAGE,
                reverse: false,
                cursor: None,
                include_system: false,
            };
            loop {
                match kv.scan(&scan).await {
                    Ok(page) => {
                        if tx.unbounded_send(Ok(page.entries)).is_err() {
                            return;
                        }
                        match page.cursor {
                            Some(cursor) => scan.cursor = Some(cursor),
                            None => return,
                        }
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(Err(e));
                        return;
                    }
                }
            }
        });
        let conn = self.conn.clone();
        runtime::spawn_db(
            cx,
            async move {
                match conn.key_value() {
                    Some(kv) => kv.key_count().await.ok(),
                    None => None,
                }
            },
            |this, total, cx| {
                this.total = total;
                cx.notify();
            },
        )
        .detach();
        self._scan = Some(cx.spawn(async move |this, cx| {
            while let Some(batch) = rx.next().await {
                let alive = this.update(cx, |tree, cx| {
                    match batch {
                        Ok(entries) => {
                            for entry in entries {
                                if tree.seen.insert(entry.key.clone()) {
                                    tree.groups
                                        .entry(group_of(&entry.key))
                                        .or_default()
                                        .push(entry);
                                }
                            }
                        }
                        Err(e) => tree.error = Some(e.to_string()),
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |tree, cx| {
                tree.scanning = false;
                for keys in tree.groups.values_mut() {
                    keys.sort_by(|a, b| natural(&a.key.to_display(), &b.key.to_display()));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Removes a deleted key from the tree.
    pub(crate) fn forget(&mut self, key: &KeyBytes, cx: &mut Context<Self>) {
        if self.seen.remove(key)
            && let Some(keys) = self.groups.get_mut(&group_of(key))
        {
            keys.retain(|e| &e.key != key);
        }
        cx.notify();
    }

    fn key_row(&self, entry: &KeyEntry, indent: f32, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.as_ref() == Some(&entry.key);
        let key = entry.key.clone();
        div()
            .id(SharedString::from(format!(
                "key-{}",
                entry.key.to_display()
            )))
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(28.))
            .pl(px(indent))
            .pr(px(8.))
            .rounded(px(8.))
            .cursor_pointer()
            .text_color(c(if selected { theme::TEXT } else { theme::MUTED }))
            .when(selected, |d| d.bg(c(theme::SELECTION)))
            .hover(|d| d.bg(c(theme::SELECTION)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.5))
                    .child(SharedString::from(entry.key.to_display())),
            )
            .child(kind_badge(entry.kind))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(key.clone());
                cx.emit(OpenKey(key.clone()));
                cx.notify();
            }))
            .into_any_element()
    }
}

/// Orders `cart:9` before `cart:10`.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| {
        let digits = s.len() - s.chars().rev().take_while(char::is_ascii_digit).count();
        (
            s[..digits].to_owned(),
            s[digits..].parse::<u64>().unwrap_or(0),
        )
    };
    split(a).cmp(&split(b))
}

impl Render for KeyTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("KeyTree::render").entered();
        let database = self
            .conn
            .info()
            .database
            .clone()
            .unwrap_or_else(|| "keys".into());
        let header = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .bg(c(theme::BG))
            .border_b_1()
            .border_color(c(theme::LINE))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::FAINT))
                    .child("DATABASE"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .bg(c(theme::RAISED))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .child(ui::icon("database", 14., theme::ACCENT_TEXT))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .child(SharedString::from(database)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(30.))
                    .pl(px(9.))
                    .pr(px(4.))
                    .rounded(px(8.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .child(ui::icon("search", 13., theme::FAINT))
                    .child(div().flex_1().child(ui::input(&self.pattern, 12.5))),
            );

        let mut nav = div()
            .id("key-tree")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(px(1.))
            .px(px(6.))
            .py(px(8.));
        for (group, keys) in &self.groups {
            if group.is_empty() {
                for entry in keys.iter().take(*self.shown.get("").unwrap_or(&SHOWN_STEP)) {
                    nav = nav.child(self.key_row(entry, 8., cx));
                }
                continue;
            }
            let open = self.expanded.contains(group);
            let name = group.clone();
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("group-{group}")))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_color(c(theme::TEXT))
                    .hover(|d| d.bg(c(theme::SELECTION)))
                    .child(ui::icon(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        13.,
                        theme::MUTED,
                    ))
                    .child(ui::icon("folder", 14., theme::MUTED))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .child(SharedString::from(group.clone())),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .text_size(px(11.5))
                            .text_color(c(theme::FAINT))
                            .child(SharedString::from(ui::thousands(keys.len() as u64))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&name) {
                            this.expanded.insert(name.clone());
                        }
                        cx.notify();
                    })),
            );
            if open {
                let shown = *self.shown.get(group).unwrap_or(&FIRST_SHOWN);
                for entry in keys.iter().take(shown) {
                    nav = nav.child(self.key_row(entry, 24., cx));
                }
                if keys.len() > shown {
                    let name = group.clone();
                    nav = nav.child(
                        div()
                            .id(SharedString::from(format!("more-{group}")))
                            .flex()
                            .items_center()
                            .h(px(28.))
                            .pl(px(24.))
                            .cursor_pointer()
                            .text_size(px(12.5))
                            .text_color(c(theme::MUTED))
                            .child(SharedString::from(format!(
                                "… {} more",
                                ui::thousands((keys.len() - shown) as u64)
                            )))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                *this.shown.entry(name.clone()).or_insert(FIRST_SHOWN) +=
                                    SHOWN_STEP;
                                cx.notify();
                            })),
                    );
                }
            }
        }

        let total = self.total.unwrap_or(self.seen.len() as u64);
        let footer = div()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(c(theme::LINE))
            .text_size(px(12.))
            .text_color(c(match &self.error {
                Some(_) => theme::DELETED_TEXT,
                None => theme::FAINT,
            }))
            .child(SharedString::from(match (&self.error, self.scanning) {
                (Some(e), _) => e.clone(),
                (None, true) => format!(
                    "Scanning… {} of {} keys",
                    ui::thousands(self.seen.len() as u64),
                    ui::thousands(total)
                ),
                (None, false) => format!(
                    "{} keys · grouped by \u{201c}:\u{201d} · SCAN, 1k per page",
                    ui::thousands(total)
                ),
            }));

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
}

/// What the key view tells the session.
pub(crate) enum KeyEvent {
    Deleted(KeyBytes),
    Renamed,
}

/// One key: header with type, TTL and size, and a per-type editor.
pub(crate) struct KeyView {
    conn: Arc<dyn Connection>,
    pub(crate) key: KeyBytes,
    value: Option<KvValue>,
    ttl: Option<(Duration, Instant)>,
    error: Option<String>,
    filter: Entity<InputState>,
    editing: Option<(usize, Entity<InputState>)>,
    adding: Option<(Entity<InputState>, Entity<InputState>)>,
    ttl_input: Option<Entity<InputState>>,
    rename_input: Option<Entity<InputState>>,
    string_input: Option<Entity<gpui_component::input::EditorState>>,
    /// String value waiting for an editor (created in `render`, which has a window).
    string_seed: Option<String>,
    confirm_delete: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<KeyEvent> for KeyView {}

/// `1752s` → `29m 12s`.
fn ttl_text(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 86_400, (s % 86_400) / 3_600, (s % 3_600) / 60, s % 60) {
        (0, 0, 0, sec) => format!("{sec}s"),
        (0, 0, m, sec) => format!("{m}m {sec}s"),
        (0, h, m, _) => format!("{h}h {m}m"),
        (d, h, _, _) => format!("{d}d {h}h"),
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Rows a value shows: (cells, editable column).
fn rows(value: &KvValue) -> Vec<Vec<String>> {
    match value {
        KvValue::Hash(f) => f
            .iter()
            .map(|f| vec![text(&f.field), text(&f.value)])
            .collect(),
        KvValue::List(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| vec![i.to_string(), text(v)])
            .collect(),
        KvValue::Set(items) => items.iter().map(|v| vec![text(v)]).collect(),
        KvValue::ZSet(m) => m
            .iter()
            .map(|m| vec![text(&m.member), m.score.to_string()])
            .collect(),
        KvValue::Stream(e) => e
            .iter()
            .map(|e| {
                let fields: Vec<String> = e
                    .fields
                    .iter()
                    .map(|f| format!("{}={}", text(&f.field), text(&f.value)))
                    .collect();
                vec![e.id.clone(), fields.join("  ")]
            })
            .collect(),
        KvValue::String(v) => vec![vec![text(v)]],
        _ => Vec::new(),
    }
}

fn headers(value: &KvValue) -> &'static [&'static str] {
    match value {
        KvValue::Hash(_) => &["Field", "Value"],
        KvValue::List(_) => &["Index", "Value"],
        KvValue::Set(_) => &["Member"],
        KvValue::ZSet(_) => &["Member", "Score"],
        KvValue::Stream(_) => &["ID", "Fields"],
        _ => &["Value"],
    }
}

fn size(value: &KvValue) -> (usize, u64) {
    let b = |v: &Vec<u8>| v.len() as u64;
    match value {
        KvValue::String(v) => (1, b(v)),
        KvValue::Hash(f) => (f.len(), f.iter().map(|f| b(&f.field) + b(&f.value)).sum()),
        KvValue::List(i) | KvValue::Set(i) => (i.len(), i.iter().map(b).sum()),
        KvValue::ZSet(m) => (m.len(), m.iter().map(|m| b(&m.member) + 8).sum()),
        KvValue::Stream(e) => (
            e.len(),
            e.iter()
                .map(|e| {
                    e.fields
                        .iter()
                        .map(|f| b(&f.field) + b(&f.value))
                        .sum::<u64>()
                })
                .sum(),
        ),
        _ => (0, 0),
    }
}

impl KeyView {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        key: KeyBytes,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter fields"));
        let sub = cx.subscribe_in(&filter, window, |_, _, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut view = Self {
            conn,
            key,
            value: None,
            ttl: None,
            error: None,
            filter,
            editing: None,
            adding: None,
            ttl_input: None,
            rename_input: None,
            string_input: None,
            string_seed: None,
            confirm_delete: false,
            _tasks: Vec::new(),
            _subscriptions: vec![sub],
        };
        view.reload(cx);
        view.tick(cx);
        view
    }

    /// Re-renders every second so the TTL counts down.
    fn tick(&mut self, cx: &mut Context<Self>) {
        self._tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update(cx, |v, cx| {
                        if v.ttl.is_some() {
                            cx.notify()
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let key = self.key.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                let kv = conn
                    .key_value()
                    .ok_or_else(|| Error::unsupported("key-value"))?;
                let value = kv.get(&key).await?;
                let ttl = match conn.key_expiry() {
                    Some(e) if value.is_some() => e.ttl(&key).await.ok().flatten(),
                    _ => None,
                };
                Ok::<_, Error>((value, ttl))
            },
            move |this, result, cx| {
                match result {
                    Ok((value, ttl)) => {
                        this.ttl = ttl.map(|d| (d, Instant::now()));
                        if let Some(KvValue::String(bytes)) = &value {
                            this.string_seed = Some(text(bytes));
                        }
                        this.value = value;
                        if this.value.is_none() {
                            this.error = Some("This key no longer exists.".into());
                        }
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    /// Writes a whole new value and reloads.
    fn write(&mut self, value: KvValue, cx: &mut Context<Self>) {
        let conn = self.conn.clone();
        let key = self.key.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.key_value() {
                    Some(kv) => kv.set(&key, &value).await,
                    None => Err(Error::unsupported("key-value")),
                }
            },
            move |this, result, cx| {
                match result {
                    Ok(()) => {
                        this.error = None;
                        this.reload(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    /// Applies a cell edit (`col` is the editable column) to a copy of the value.
    fn edited(&self, row: usize, new: String) -> Option<KvValue> {
        let b = |s: &str| s.as_bytes().to_vec();
        let mut value = self.value.clone()?;
        match &mut value {
            KvValue::Hash(f) => f.get_mut(row)?.value = b(&new),
            KvValue::List(items) => *items.get_mut(row)? = b(&new),
            KvValue::Set(items) => *items.get_mut(row)? = b(&new),
            KvValue::ZSet(m) => m.get_mut(row)?.score = new.trim().parse().ok()?,
            _ => return None,
        }
        Some(value)
    }

    fn removed(&self, row: usize) -> Option<KvValue> {
        let mut value = self.value.clone()?;
        match &mut value {
            KvValue::Hash(f) => drop(f.remove(row)),
            KvValue::List(i) | KvValue::Set(i) => drop(i.remove(row)),
            KvValue::ZSet(m) => drop(m.remove(row)),
            KvValue::Stream(e) => drop(e.remove(row)),
            _ => return None,
        }
        Some(value)
    }

    fn added(&self, first: String, second: String) -> Option<KvValue> {
        let b = |s: &str| s.as_bytes().to_vec();
        let mut value = self.value.clone()?;
        match &mut value {
            KvValue::Hash(f) => match f.iter_mut().find(|f| f.field == b(&first)) {
                Some(existing) => existing.value = b(&second),
                None => f.push(HashField {
                    field: b(&first),
                    value: b(&second),
                }),
            },
            KvValue::List(i) => i.push(b(&first)),
            KvValue::Set(i) if !i.contains(&b(&first)) => i.push(b(&first)),
            KvValue::Set(_) => {}
            KvValue::ZSet(m) => m.push(ScoredMember {
                member: b(&first),
                score: second.trim().parse().unwrap_or(0.0),
            }),
            _ => return None,
        }
        Some(value)
    }

    fn start_edit(
        &mut self,
        row: usize,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        let focus = input.clone();
        cx.defer_in(window, move |_, window, cx| {
            focus.update(cx, |s, cx| s.focus(window, cx))
        });
        self._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            move |this, input, e: &InputEvent, _window, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().to_string();
                    this.editing = None;
                    if let Some(value) = this.edited(row, text) {
                        this.write(value, cx);
                    }
                    cx.notify();
                }
            },
        ));
        self.editing = Some((row, input));
        cx.notify();
    }

    fn open_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first_hint = match self.value {
            Some(KvValue::Hash(_)) => "field",
            _ => "value",
        };
        let first = cx.new(|cx| InputState::new(window, cx).placeholder(first_hint));
        let second = cx.new(|cx| {
            InputState::new(window, cx).placeholder(match self.value {
                Some(KvValue::ZSet(_)) => "score",
                _ => "value",
            })
        });
        first.update(cx, |s, cx| s.focus(window, cx));
        self.adding = Some((first, second));
        cx.notify();
    }

    fn submit_adding(&mut self, cx: &mut Context<Self>) {
        if let Some((first, second)) = self.adding.take() {
            let (a, b) = (
                first.read(cx).value().to_string(),
                second.read(cx).value().to_string(),
            );
            if !a.is_empty()
                && let Some(value) = self.added(a, b)
            {
                self.write(value, cx);
            }
        }
        cx.notify();
    }

    fn submit_ttl(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.ttl_input.take() else {
            return;
        };
        let text = input.read(cx).value().trim().to_owned();
        let ttl = if text.is_empty() {
            None
        } else {
            match text.parse::<u64>() {
                Ok(s) => Some(Duration::from_secs(s)),
                Err(_) => {
                    self.error = Some("TTL is a number of seconds (empty removes it).".into());
                    cx.notify();
                    return;
                }
            }
        };
        let conn = self.conn.clone();
        let key = self.key.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.key_expiry() {
                    Some(e) => e.set_ttl(&key, ttl).await,
                    None => Err(Error::unsupported("TTL")),
                }
            },
            move |this, result, cx| {
                match result {
                    Ok(()) => {
                        this.reload(cx);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    fn submit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.rename_input.take() else {
            return;
        };
        let to = KeyBytes::from(input.read(cx).value().trim());
        if to.as_bytes().is_empty() || to == self.key {
            cx.notify();
            return;
        }
        let conn = self.conn.clone();
        let from = self.key.clone();
        let target = to.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.key_value() {
                    Some(kv) => kv.rename(&from, &target).await,
                    None => Err(Error::unsupported("key-value")),
                }
            },
            move |this, result, cx| {
                match result {
                    Ok(()) => {
                        this.key = to.clone();
                        cx.emit(KeyEvent::Renamed);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if !self.confirm_delete {
            self.confirm_delete = true;
            cx.notify();
            return;
        }
        let conn = self.conn.clone();
        let key = self.key.clone();
        self._tasks.push(runtime::spawn_db(
            cx,
            async move {
                match conn.key_value() {
                    Some(kv) => kv.delete(&key).await,
                    None => Err(Error::unsupported("key-value")),
                }
            },
            |this, result, cx| {
                match result {
                    Ok(_) => cx.emit(KeyEvent::Deleted(this.key.clone())),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
    }

    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.value.as_ref().map(KvValue::kind);
        let (count, bytes) = self.value.as_ref().map(size).unwrap_or_default();
        let unit = match kind {
            Some(KeyKind::Hash) => "fields",
            Some(KeyKind::String) => "value",
            Some(KeyKind::Stream) => "entries",
            Some(KeyKind::ZSet | KeyKind::Set) => "members",
            _ => "items",
        };
        let remaining = self.ttl.map(|(d, at)| d.saturating_sub(at.elapsed()));
        let inline = |input: &Entity<InputState>, width: f32| {
            div()
                .w(px(width))
                .h(px(28.))
                .rounded(px(8.))
                .bg(c(theme::BG))
                .border_1()
                .border_color(c(theme::ACCENT))
                .child(ui::input(input, 12.5))
        };
        let _ = window;
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(14.))
            .border_b_1()
            .border_color(c(theme::LINE))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(self.key.to_display())),
            )
            .child(kind_badge(kind))
            .when_some(
                remaining.filter(|_| self.conn.key_expiry().is_some()),
                |d, r| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.5))
                            .text_color(c(theme::MUTED))
                            .child(ui::icon("clock", 13., theme::MUTED))
                            .child(SharedString::from(format!("TTL {}", ttl_text(r)))),
                    )
                },
            )
            .when(self.value.is_some(), |d| {
                d.child(div().text_size(px(12.5)).text_color(c(theme::FAINT)).child(
                    SharedString::from(format!(
                        "{count} {unit} · {}",
                        ui::bytes(bytes).replace(" bytes", " B")
                    )),
                ))
            })
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .when_some(self.ttl_input.as_ref(), |d, input| {
                        d.child(inline(input, 120.)).child(
                            ui::secondary_button("apply-ttl", "Apply")
                                .on_click(cx.listener(|this, _, _window, cx| this.submit_ttl(cx))),
                        )
                    })
                    .when(
                        self.ttl_input.is_none() && self.conn.key_expiry().is_some(),
                        |d| {
                            d.child(
                                ui::secondary_button("set-ttl", "Set TTL")
                                    .flex_row_reverse()
                                    .child(ui::icon("clock", 13., theme::TEXT))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let input = cx.new(|cx| {
                                            InputState::new(window, cx).placeholder("seconds")
                                        });
                                        input.update(cx, |s, cx| s.focus(window, cx));
                                        this.ttl_input = Some(input);
                                        cx.notify();
                                    })),
                            )
                        },
                    )
                    .when_some(self.rename_input.as_ref(), |d, input| {
                        d.child(inline(input, 200.)).child(
                            ui::secondary_button("apply-rename", "Rename")
                                .on_click(cx.listener(|this, _, _, cx| this.submit_rename(cx))),
                        )
                    })
                    .when(self.rename_input.is_none(), |d| {
                        d.child(
                            ui::secondary_button("rename", "Rename").on_click(cx.listener(
                                |this, _, window, cx| {
                                    let current = this.key.to_display();
                                    let input = cx.new(|cx| {
                                        InputState::new(window, cx).default_value(current)
                                    });
                                    input.update(cx, |s, cx| s.focus(window, cx));
                                    this.rename_input = Some(input);
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .child(
                        ui::ghost_button(
                            "delete-key",
                            if self.confirm_delete {
                                "Delete for good?"
                            } else {
                                "Delete key"
                            },
                        )
                        .flex_row_reverse()
                        .child(ui::icon("trash", 13., theme::DELETED_TEXT))
                        .text_color(c(theme::DELETED_TEXT))
                        .border_1()
                        .border_color(gpui::rgb(0xf1bcb7))
                        .when(self.confirm_delete, |b| b.bg(c(theme::DELETED_BG)))
                        .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    ),
            )
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(value) = &self.value else {
            return div()
                .p(px(16.))
                .text_size(px(13.))
                .text_color(c(theme::DELETED_TEXT))
                .children(self.error.clone().map(SharedString::from))
                .into_any_element();
        };
        if let (KvValue::String(_), Some(input)) = (value, &self.string_input) {
            return div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(16.))
                .child(
                    div()
                        .h(px(240.))
                        .p(px(4.))
                        .rounded(px(8.))
                        .bg(c(theme::BG))
                        .border_1()
                        .border_color(c(theme::LINE))
                        .font_family(theme::mono(cx))
                        .child(
                            gpui_component::input::Editor::new(input)
                                .appearance(false)
                                .h_full(),
                        ),
                )
                .child(
                    div().child(
                        ui::primary_button("save-string", "Save").on_click(cx.listener(
                            |this, _, _window, cx| {
                                if let Some(input) = &this.string_input {
                                    let text = input.read(cx).value().to_string();
                                    this.write(KvValue::String(text.into_bytes()), cx);
                                }
                            },
                        )),
                    ),
                )
                .into_any_element();
        }
        let filter = self.filter.read(cx).value().to_lowercase();
        let heads = headers(value);
        let editable_col = heads.len() - 1;
        let can_edit = !matches!(value, KvValue::Stream(_));
        let widths = if heads.len() == 1 { vec![] } else { vec![287.] };
        let cell = |i: usize| {
            let base = div()
                .px(px(12.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis();
            match widths.get(i) {
                Some(w) => base.w(px(*w)).flex_none(),
                None => base.flex_1().min_w_0(),
            }
        };
        let mut table = div().flex().flex_col().child(
            div()
                .flex()
                .items_center()
                .h(px(33.))
                .bg(c(theme::PANEL))
                .border_b_1()
                .border_color(c(theme::LINE))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(theme::MUTED))
                .children(heads.iter().enumerate().map(|(i, h)| cell(i).child(*h)))
                .child(div().w(px(36.)).flex_none()),
        );
        for (row, cells) in rows(value).into_iter().enumerate() {
            if !filter.is_empty() && !cells[0].to_lowercase().contains(&filter) {
                continue;
            }
            let current = cells[editable_col].clone();
            let editing = self
                .editing
                .as_ref()
                .filter(|(r, _)| *r == row)
                .map(|(_, i)| i.clone());
            table = table.child(
                div()
                    .id(("kv-row", row))
                    .flex()
                    .items_center()
                    .h(px(31.))
                    .bg(c(theme::BG))
                    .border_b_1()
                    .border_color(c(theme::LINE))
                    .text_size(px(12.5))
                    .children(cells.into_iter().enumerate().map(|(i, text)| {
                        if i == editable_col
                            && let Some(input) = &editing
                        {
                            return cell(i)
                                .child(
                                    div()
                                        .h(px(26.))
                                        .rounded(px(8.))
                                        .border_2()
                                        .border_color(c(theme::ACCENT))
                                        .child(ui::input(input, 12.5)),
                                )
                                .into_any_element();
                        }
                        let current = current.clone();
                        cell(i)
                            .id(("kv-cell", row * 4 + i))
                            .child(SharedString::from(text))
                            .when(i == editable_col && can_edit, |d| {
                                d.on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(
                                        move |this, e: &gpui::MouseDownEvent, window, cx| {
                                            if e.click_count == 2 {
                                                this.start_edit(row, current.clone(), window, cx);
                                            }
                                        },
                                    ),
                                )
                            })
                            .into_any_element()
                    }))
                    .child(div().w(px(36.)).flex_none().flex().justify_center().child(
                        ui::icon_button(("kv-remove", row), "trash", 24., 13.).on_click(
                            cx.listener(move |this, _, _window, cx| {
                                if let Some(value) = this.removed(row) {
                                    this.write(value, cx);
                                }
                            }),
                        ),
                    )),
            );
        }
        if let Some((first, second)) = &self.adding {
            let field = |input: &Entity<InputState>| {
                div()
                    .h(px(26.))
                    .rounded(px(8.))
                    .bg(c(theme::BG))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .child(ui::input(input, 12.5))
            };
            let two = heads.len() == 2 && !matches!(value, KvValue::List(_));
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(36.))
                    .bg(c(theme::NEW_BG))
                    .border_b_1()
                    .border_color(c(theme::LINE))
                    .child(cell(0).child(field(first)))
                    .when(two, |d| d.child(cell(1).child(field(second))))
                    .when(!two && heads.len() == 2, |d| d.child(cell(1)))
                    .child(
                        div().w(px(36.)).flex_none().pr(px(4.)).child(
                            ui::secondary_button("submit-add", "Add")
                                .h(px(24.))
                                .on_click(
                                    cx.listener(|this, _, _window, cx| this.submit_adding(cx)),
                                ),
                        ),
                    ),
            );
        }
        table.into_any_element()
    }
}

impl Render for KeyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("KeyView::render").entered();
        if let Some(seed) = self.string_seed.take() {
            self.string_input =
                Some(cx.new(|cx| {
                    gpui_component::input::EditorState::new(window, cx).default_value(seed)
                }));
        }
        let kind = self.value.as_ref().map(KvValue::kind);
        let collection = !matches!(kind, Some(KeyKind::String) | None);
        let add_label = match kind {
            Some(KeyKind::Hash) => "Add field",
            Some(KeyKind::ZSet | KeyKind::Set) => "Add member",
            _ => "Add item",
        };
        let engine = self
            .conn
            .info()
            .server_version
            .split_whitespace()
            .next()
            .unwrap_or("the store")
            .to_owned();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_header(window, cx))
            .when(collection, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(16.))
                        .py(px(8.))
                        .border_b_1()
                        .border_color(c(theme::LINE))
                        .child(
                            div()
                                .w(px(220.))
                                .h(px(28.))
                                .rounded(px(8.))
                                .bg(c(theme::BG))
                                .border_1()
                                .border_color(c(theme::LINE))
                                .child(ui::input(&self.filter, 12.5)),
                        )
                        .when(!matches!(kind, Some(KeyKind::Stream)), |d| {
                            d.child(
                                div().ml_auto().child(
                                    ui::ghost_button("add-item", add_label)
                                        .flex_row_reverse()
                                        .child(ui::icon("plus", 14., theme::MUTED))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_adding(window, cx)
                                        })),
                                ),
                            )
                        }),
                )
            })
            .child(
                div()
                    .id("kv-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_body(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(37.))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(c(theme::LINE))
                    .bg(c(theme::PANEL))
                    .text_size(px(12.))
                    .text_color(c(if self.error.is_some() && self.value.is_some() {
                        theme::DELETED_TEXT
                    } else {
                        theme::MUTED
                    }))
                    .child(SharedString::from(
                        match (&self.error, self.value.is_some()) {
                            (Some(e), true) => e.clone(),
                            _ => format!("Edits write straight to {engine}"),
                        },
                    )),
            )
    }
}

/// Right panel: memory by prefix and keys by type, from `MemoryStats`.
pub(crate) struct MemoryPanel {
    report: Option<MemoryReport>,
    error: Option<String>,
    _task: Option<Task<()>>,
}

impl MemoryPanel {
    pub(crate) fn new(conn: Arc<dyn Connection>, cx: &mut Context<Self>) -> Self {
        let task = runtime::spawn_db(
            cx,
            async move {
                match conn.memory_stats() {
                    Some(m) => m.memory_report().await,
                    None => Err(Error::unsupported("memory stats")),
                }
            },
            |this: &mut Self, result, cx| {
                match result {
                    Ok(report) => this.report = Some(report),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        );
        Self {
            report: None,
            error: None,
            _task: Some(task),
        }
    }
}

fn bar_row(label: String, fraction: f32, value: String, color: u32) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(22.))
        .text_size(px(12.))
        .child(
            div()
                .w(px(96.))
                .flex_none()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .flex_1()
                .h(px(8.))
                .rounded(px(2.))
                .bg(c(theme::RAISED))
                .child(
                    div()
                        .h_full()
                        .rounded(px(2.))
                        .bg(c(color))
                        .w(gpui::relative(fraction.clamp(0.02, 1.0))),
                ),
        )
        .child(
            div()
                .w(px(56.))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(c(theme::MUTED))
                .child(SharedString::from(value)),
        )
        .into_any_element()
}

impl Render for MemoryPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("MemoryPanel::render").entered();
        let title = |t: &'static str| {
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(t)
        };
        let mut body = div().flex().flex_col().gap(px(14.)).p(px(16.));
        match (&self.report, &self.error) {
            (Some(r), _) => {
                let top: Vec<_> = r.by_prefix.iter().take(4).collect();
                let max = top.iter().map(|p| p.bytes).max().unwrap_or(1).max(1) as f32;
                let kmax = r.by_kind.iter().map(|k| k.keys).max().unwrap_or(1).max(1) as f32;
                body = body
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(title("Memory by prefix"))
                            .child(div().text_size(px(12.)).text_color(c(theme::FAINT)).child(
                                SharedString::from(format!(
                                    "{} used{}",
                                    ui::bytes(r.used_bytes),
                                    if r.sampled {
                                        " · sampled with MEMORY USAGE"
                                    } else {
                                        ""
                                    }
                                )),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(0.))
                            .children(top.iter().map(|p| {
                                bar_row(
                                    p.prefix.clone(),
                                    p.bytes as f32 / max,
                                    ui::bytes(p.bytes),
                                    0xa597d8,
                                )
                            })),
                    )
                    .child(div().h(px(1.)).bg(c(theme::LINE)))
                    .child(title("Keys by type"))
                    .child(div().flex().flex_col().children(r.by_kind.iter().map(|k| {
                        bar_row(
                            k.kind.label().into(),
                            k.keys as f32 / kmax,
                            ui::thousands(k.keys),
                            theme::ACCENT,
                        )
                    })));
            }
            (None, Some(e)) => {
                body = body.child(
                    div()
                        .text_color(c(theme::DELETED_TEXT))
                        .child(SharedString::from(e.clone())),
                )
            }
            (None, None) => {
                body = body.child(div().text_color(c(theme::FAINT)).child("Sampling memory…"))
            }
        }
        div()
            .id("memory-panel")
            .flex_none()
            .w(px(332.))
            .h_full()
            .overflow_y_scroll()
            .bg(c(theme::PANEL))
            .border_l_1()
            .border_color(c(theme::LINE))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn ttl_is_compact() {
        assert_eq!(super::ttl_text(Duration::from_secs(1752)), "29m 12s");
        assert_eq!(super::ttl_text(Duration::from_secs(42)), "42s");
        assert_eq!(super::ttl_text(Duration::from_secs(7_300)), "2h 1m");
    }

    #[test]
    fn natural_order_for_numeric_suffixes() {
        assert!(super::natural("cart:9", "cart:10").is_lt());
    }
}
