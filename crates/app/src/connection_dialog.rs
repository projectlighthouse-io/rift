//! New / edit connection dialog. Every driver field comes from `DriverInfo.fields`, so a new
//! driver needs no UI changes.

use std::collections::BTreeMap;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    BoxShadow, Context, Entity, EventEmitter, FontWeight, KeyDownEvent, PathPromptOptions,
    SharedString, Subscription, Task, Window, div, point, px,
};
use gpui_component::input::InputState;
use types::{
    ConnectionId, DriverInfo, FieldKind, FieldSpec, SafeMode, SavedConnection, Secret, SshConfig,
    Tag, TlsMode,
};

use crate::connections::KNOWN_GROUPS;
use crate::dropdown::{Changed, Dropdown};
use crate::theme::{self, c};
use crate::{runtime, state, ui};

/// What the dialog asks the workspace to do.
pub(crate) enum DialogEvent {
    /// Saved; the workspace connects to it.
    Saved(Box<SavedConnection>),
    Deleted,
    Cancelled,
}

struct Field {
    spec: FieldSpec,
    input: Entity<InputState>,
}

/// The dialog.
pub(crate) struct ConnectionDialog {
    editing: Option<SavedConnection>,
    drivers: Vec<DriverInfo>,
    driver: Entity<Dropdown>,
    name: Entity<InputState>,
    fields: Vec<Field>,
    group: Entity<Dropdown>,
    tag: Tag,
    more: bool,
    tls: Entity<Dropdown>,
    safe_mode: Entity<Dropdown>,
    ssh_host: Entity<InputState>,
    ssh_port: Entity<InputState>,
    ssh_user: Entity<InputState>,
    ssh_key: Entity<InputState>,
    test: Option<Result<Duration, String>>,
    error: Option<String>,
    _testing: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DialogEvent> for ConnectionDialog {}

const TLS: [(&str, TlsMode); 3] = [
    ("Disable", TlsMode::Disable),
    ("Prefer", TlsMode::Prefer),
    ("Require", TlsMode::Require),
];
const SAFE: [(&str, SafeMode); 3] = [
    ("Off", SafeMode::Off),
    ("Confirm writes", SafeMode::ConfirmWrites),
    ("Read-only", SafeMode::ReadOnly),
];

fn text_input(
    value: &str,
    placeholder: &str,
    window: &mut Window,
    cx: &mut Context<ConnectionDialog>,
) -> Entity<InputState> {
    let value = value.to_owned();
    let placeholder = placeholder.to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

impl ConnectionDialog {
    /// A dialog for a new connection (`editing == None`) or editing a saved one.
    pub(crate) fn new(
        editing: Option<SavedConnection>,
        existing_groups: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let manager = state::manager(cx);
        let drivers: Vec<DriverInfo> = manager.drivers().cloned().collect();
        let driver_ix = editing
            .as_ref()
            .and_then(|e| drivers.iter().position(|d| d.id == e.driver))
            .unwrap_or(0);
        let driver = cx.new(|_| {
            Dropdown::new(
                drivers.iter().map(|d| d.name.clone().into()).collect(),
                driver_ix,
            )
        });

        let mut groups: Vec<SharedString> = KNOWN_GROUPS.iter().map(|g| (*g).into()).collect();
        for g in existing_groups {
            if !groups.iter().any(|x| x.as_ref() == g) {
                groups.push(g.into());
            }
        }
        let group_ix = editing
            .as_ref()
            .and_then(|e| e.group.as_ref())
            .and_then(|g| groups.iter().position(|x| x.as_ref() == g))
            .unwrap_or(0);
        let group = cx.new(|_| Dropdown::new(groups, group_ix));

        let tls_ix = editing
            .as_ref()
            .map_or(0, |e| TLS.iter().position(|t| t.1 == e.tls).unwrap_or(0));
        let tls = cx.new(|_| Dropdown::new(TLS.iter().map(|t| t.0.into()).collect(), tls_ix));
        let safe_ix = editing.as_ref().map_or(0, |e| {
            SAFE.iter().position(|t| t.1 == e.safe_mode).unwrap_or(0)
        });
        let safe_mode =
            cx.new(|_| Dropdown::new(SAFE.iter().map(|t| t.0.into()).collect(), safe_ix));

        let ssh = editing.as_ref().and_then(|e| e.ssh.clone());
        let name = text_input(
            editing.as_ref().map_or("", |e| e.name.as_str()),
            "local-dev",
            window,
            cx,
        );
        let ssh_host = text_input(
            ssh.as_ref().map_or("", |s| s.host.as_str()),
            "bastion.example.com",
            window,
            cx,
        );
        let ssh_port_value = ssh.as_ref().map_or("22".to_owned(), |s| s.port.to_string());
        let ssh_port = text_input(&ssh_port_value, "22", window, cx);
        let ssh_user = text_input(
            ssh.as_ref().map_or("", |s| s.user.as_str()),
            "deploy",
            window,
            cx,
        );
        let ssh_key = text_input(
            ssh.as_ref()
                .and_then(|s| s.key_path.as_deref())
                .unwrap_or(""),
            "~/.ssh/id_ed25519",
            window,
            cx,
        );

        let subscription = cx.subscribe_in(&driver, window, |this, _, _: &Changed, window, cx| {
            this.rebuild_fields(window, cx);
        });
        let mut dialog = Self {
            tag: editing.as_ref().and_then(|e| e.tag).unwrap_or(Tag::Green),
            more: false,
            editing,
            drivers,
            driver,
            name,
            fields: Vec::new(),
            group,
            tls,
            safe_mode,
            ssh_host,
            ssh_port,
            ssh_user,
            ssh_key,
            test: None,
            error: None,
            _testing: None,
            _subscriptions: vec![subscription],
        };
        dialog.rebuild_fields(window, cx);
        dialog.name.update(cx, |s, cx| s.focus(window, cx));
        dialog
    }

    fn driver_info(&self, cx: &gpui::App) -> &DriverInfo {
        &self.drivers[self.driver.read(cx).selected()]
    }

    /// Recreates inputs for the selected driver, keeping values of fields with the same key.
    fn rebuild_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous: BTreeMap<String, String> = self
            .fields
            .iter()
            .map(|f| (f.spec.key.to_string(), f.input.read(cx).value().to_string()))
            .collect();
        let saved = self
            .editing
            .as_ref()
            .filter(|e| e.driver == self.driver_info(cx).id);
        let password = saved.and_then(|e| state::manager(cx).password(e.id).ok().flatten());
        let specs = self.driver_info(cx).fields.clone();
        self.fields = specs
            .into_iter()
            .map(|spec| {
                let key = spec.key.to_string();
                let value = if spec.kind == FieldKind::Password {
                    password
                        .as_ref()
                        .map(|p| p.expose().to_owned())
                        .unwrap_or_default()
                } else {
                    saved
                        .and_then(|e| e.values.get(&spec.key).cloned())
                        .or_else(|| previous.get(&key).cloned().filter(|v| !v.is_empty()))
                        .or_else(|| spec.default.clone())
                        .unwrap_or_default()
                };
                let placeholder = spec.placeholder.clone().unwrap_or_default();
                let masked = spec.kind == FieldKind::Password;
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .masked(masked)
                        .default_value(value)
                });
                Field { spec, input }
            })
            .collect();
        self.test = None;
        cx.notify();
    }

    /// Reads the form. Errors name the first missing required field.
    fn build(&self, cx: &gpui::App) -> Result<(SavedConnection, Option<Secret>), String> {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return Err("Name is required.".into());
        }
        let mut values = BTreeMap::new();
        let mut password = None;
        for field in &self.fields {
            let value = field.input.read(cx).value().trim().to_owned();
            if field.spec.required && value.is_empty() {
                return Err(format!("{} is required.", field.spec.label));
            }
            if field.spec.kind == FieldKind::Password {
                password = Some(Secret::new(value));
            } else if !value.is_empty() {
                values.insert(field.spec.key.clone(), value);
            }
        }
        let ssh_host = self.ssh_host.read(cx).value().trim().to_owned();
        let ssh = if ssh_host.is_empty() {
            None
        } else {
            let key = self.ssh_key.read(cx).value().trim().to_owned();
            Some(SshConfig {
                host: ssh_host,
                port: self
                    .ssh_port
                    .read(cx)
                    .value()
                    .trim()
                    .parse()
                    .map_err(|_| "SSH port must be a number.")?,
                user: self.ssh_user.read(cx).value().trim().to_owned(),
                key_path: (!key.is_empty()).then_some(key),
            })
        };
        let connection = SavedConnection {
            id: self
                .editing
                .as_ref()
                .map_or_else(ConnectionId::random, |e| e.id),
            name,
            driver: self.driver_info(cx).id.clone(),
            group: Some(self.group.read(cx).value().to_string()),
            tag: Some(self.tag),
            values,
            tls: TLS[self.tls.read(cx).selected()].1,
            safe_mode: SAFE[self.safe_mode.read(cx).selected()].1,
            ssh,
        };
        Ok((connection, password))
    }

    fn run_test(&mut self, cx: &mut Context<Self>) {
        let (connection, password) = match self.build(cx) {
            Ok(built) => built,
            Err(e) => {
                self.test = Some(Err(e));
                cx.notify();
                return;
            }
        };
        let manager = state::manager(cx);
        let config = conn::ConnectionManager::config(&connection, password);
        self.test = None;
        self._testing = Some(runtime::spawn_db(
            cx,
            async move { manager.test(&config).await },
            |this, result, cx| {
                this.test = Some(result.map_err(|e| e.to_string()));
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        match self.build(cx) {
            Ok((connection, password)) => {
                match state::manager(cx).save(&connection, password.as_ref()) {
                    Ok(()) => cx.emit(DialogEvent::Saved(Box::new(connection))),
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            Err(e) => self.error = Some(e),
        }
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if let Some(editing) = &self.editing {
            match state::manager(cx).delete(editing.id) {
                Ok(()) => cx.emit(DialogEvent::Deleted),
                Err(e) => self.error = Some(e.to_string()),
            }
            cx.notify();
        }
    }

    fn browse(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let input = self.fields[ix].input.clone();
        let window_handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().to_string();
                let _ = window_handle.update(cx, |_, window, cx| {
                    input.update(cx, |s, cx| s.set_value(path, window, cx));
                });
            }
        })
        .detach();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            cx.emit(DialogEvent::Cancelled);
            cx.stop_propagation();
        }
    }
}

fn label(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(c(theme::MUTED))
        .child(text.into())
}

fn boxed(child: impl IntoElement, mono: SharedString) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .h(px(34.))
        .px(px(2.))
        .rounded(px(4.))
        .bg(c(theme::BG))
        .border_1()
        .border_color(c(theme::LINE))
        .font_family(mono)
        .text_size(px(13.))
        .child(div().flex_1().child(child))
}

fn column(label_text: impl Into<SharedString>, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .flex_1()
        .min_w_0()
        .child(label(label_text))
        .child(control)
}

fn row() -> gpui::Div {
    div().flex().gap(px(12.))
}

impl Render for ConnectionDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("ConnectionDialog::render").entered();
        let mono = theme::mono(cx);
        let title = if self.editing.is_some() {
            "Edit connection"
        } else {
            "New connection"
        };

        let mut body = div().flex().flex_col().gap(px(14.)).px(px(20.)).py(px(16.));
        body = body.child(
            row()
                .child(column("Type", self.driver.clone()))
                .child(column(
                    "Name",
                    boxed(ui::input(&self.name, 13.), mono.clone()),
                )),
        );

        let basic: Vec<usize> = (0..self.fields.len())
            .filter(|&i| !self.fields[i].spec.advanced)
            .collect();
        let advanced: Vec<usize> = (0..self.fields.len())
            .filter(|&i| self.fields[i].spec.advanced)
            .collect();
        let field_control = |this: &Self, ix: usize, cx: &mut Context<Self>| {
            let field = &this.fields[ix];
            let input = boxed(ui::input(&field.input, 13.), mono.clone());
            if field.spec.kind == FieldKind::File {
                row()
                    .gap(px(8.))
                    .child(div().flex_1().child(input))
                    .child(
                        ui::secondary_button(("browse", ix), "Browse…")
                            .h(px(34.))
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.browse(ix, window, cx)),
                            ),
                    )
                    .into_any_element()
            } else {
                input.into_any_element()
            }
        };

        // Basic fields two per row; Host + Port share a row with a narrow port; Group fills
        // the last odd slot.
        let mut pending: Vec<gpui::AnyElement> = Vec::new();
        let mut i = 0;
        while i < basic.len() {
            let ix = basic[i];
            let spec = &self.fields[ix].spec;
            let next_is_port = basic
                .get(i + 1)
                .is_some_and(|&n| self.fields[n].spec.kind == FieldKind::Port);
            if spec.kind == FieldKind::Host && next_is_port {
                let port_ix = basic[i + 1];
                body = body.child(
                    row()
                        .child(column(spec.label.clone(), field_control(self, ix, cx)))
                        .child(div().w(px(96.)).flex_none().child(column(
                            self.fields[port_ix].spec.label.clone(),
                            field_control(self, port_ix, cx),
                        ))),
                );
                i += 2;
                continue;
            }
            pending
                .push(column(spec.label.clone(), field_control(self, ix, cx)).into_any_element());
            if pending.len() == 2 {
                body = body.child(row().children(pending.drain(..)));
            }
            i += 1;
        }
        pending.push(column("Group", self.group.clone()).into_any_element());
        if pending.len() == 1 {
            pending.push(div().flex_1().into_any_element());
        }
        body = body.child(row().children(pending.drain(..)));

        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(label("Tag"))
                .child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .children(Tag::ALL.iter().map(|&tag| {
                            let selected = self.tag == tag;
                            div()
                                .id(SharedString::from(format!("tag-{tag:?}")))
                                .relative()
                                .size(px(20.))
                                .cursor_pointer()
                                .child(ui::tag_square(tag, 20.).rounded(px(3.)))
                                .when(selected, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top(px(-4.))
                                            .left(px(-4.))
                                            .size(px(28.))
                                            .rounded(px(5.))
                                            .border_2()
                                            .border_color(c(theme::MUTED)),
                                    )
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.tag = tag;
                                    cx.notify();
                                }))
                        })),
                ),
        );

        body = body.child(
            div()
                .id("more-options")
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.5))
                .text_color(c(theme::MUTED))
                .cursor_pointer()
                .child(ui::icon(
                    if self.more {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    14.,
                    theme::MUTED,
                ))
                .child("More options (SSH, TLS, read-only)")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.more = !this.more;
                    cx.notify();
                })),
        );
        if self.more {
            body = body
                .child(
                    row()
                        .child(column("TLS", self.tls.clone()))
                        .child(column("Safe mode", self.safe_mode.clone())),
                )
                .children(advanced.chunks(2).map(|pair| {
                    row().children(pair.iter().map(|&ix| {
                        column(
                            self.fields[ix].spec.label.clone(),
                            field_control(self, ix, cx),
                        )
                    }))
                }))
                .child(
                    row()
                        .child(column(
                            "SSH host",
                            boxed(ui::input(&self.ssh_host, 13.), mono.clone()),
                        ))
                        .child(div().w(px(96.)).flex_none().child(column(
                            "SSH port",
                            boxed(ui::input(&self.ssh_port, 13.), mono.clone()),
                        ))),
                )
                .child(
                    row()
                        .child(column(
                            "SSH user",
                            boxed(ui::input(&self.ssh_user, 13.), mono.clone()),
                        ))
                        .child(column(
                            "SSH key",
                            boxed(ui::input(&self.ssh_key, 13.), mono.clone()),
                        )),
                );
        }

        let test_status = match (&self.test, &self.error) {
            (_, Some(e)) => Some(
                div()
                    .text_size(px(12.))
                    .text_color(c(theme::DELETED_TEXT))
                    .child(SharedString::from(e.clone())),
            ),
            (Some(Ok(d)), None) => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(c(theme::NEW_TEXT))
                    .child(ui::icon("check", 13., theme::NEW_TEXT))
                    .child(SharedString::from(format!("OK · {} ms", d.as_millis()))),
            ),
            (Some(Err(e)), None) => Some(
                div()
                    .text_size(px(12.))
                    .text_color(c(theme::DELETED_TEXT))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .max_w(px(220.))
                    .child(SharedString::from(e.clone())),
            ),
            (None, None) => None,
        };

        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(20.))
            .py(px(14.))
            .border_t_1()
            .border_color(c(theme::LINE))
            .child(
                ui::secondary_button("test", "Test")
                    .h(px(32.))
                    .px(px(14.))
                    .flex_row_reverse()
                    .child(ui::icon("check", 14., theme::TEXT))
                    .on_click(cx.listener(|this, _, _, cx| this.run_test(cx))),
            )
            .children(test_status)
            .child(div().ml_auto())
            .when(self.editing.is_some(), |d| {
                d.child(
                    ui::ghost_button("delete", "Delete")
                        .h(px(32.))
                        .text_color(c(theme::DELETED_TEXT))
                        .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                )
            })
            .child(
                ui::ghost_button("cancel", "Cancel")
                    .h(px(32.))
                    .px(px(12.))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Cancelled))),
            )
            .child(
                ui::primary_button("save", "Save & connect")
                    .h(px(32.))
                    .px(px(14.))
                    .rounded(px(4.))
                    .text_size(px(13.))
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            );

        let dialog =
            div()
                .id("connection-dialog")
                .occlude()
                .flex()
                .flex_col()
                .w_full()
                .max_w(px(520.))
                .bg(c(theme::BG))
                .border_1()
                .border_color(c(theme::LINE))
                .rounded(px(6.))
                .shadow(vec![BoxShadow {
                    color: gpui::hsla(220. / 360., 0.43, 0.11, 0.18),
                    offset: point(px(0.), px(24.)),
                    blur_radius: px(64.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(20.))
                        .pt(px(16.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().ml_auto().child(
                            ui::icon_button("close-dialog", "close", 28., 15.).on_click(
                                cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Cancelled)),
                            ),
                        )),
                )
                .child(body)
                .child(footer);

        div()
            .id("dialog-screen")
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.))
            .capture_key_down(cx.listener(Self::on_key))
            .child(ui::dot_grid())
            .child(dialog)
    }
}
