//! Keyboard actions and their default bindings.

use gpui::{App, KeyBinding, actions};

actions!(
    rift,
    [
        /// Open the New connection dialog.
        NewConnection,
        /// Edit the highlighted connection.
        EditConnection,
        /// Save pending changes.
        SaveChanges,
        /// Open the command palette.
        OpenPalette,
        /// Open or close the connection switcher.
        ToggleSwitcher,
    ]
);

/// `cmd!("n")` is `cmd-n` on macOS and `ctrl-n` elsewhere.
macro_rules! cmd {
    ($key:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $key)
        } else {
            concat!("ctrl-", $key)
        }
    };
}

/// Registers default key bindings.
pub(crate) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(cmd!("n"), NewConnection, Some("Connections")),
        KeyBinding::new(cmd!("e"), EditConnection, Some("Connections")),
        KeyBinding::new(cmd!("s"), SaveChanges, Some("TableView")),
        KeyBinding::new(cmd!("k"), OpenPalette, None),
        KeyBinding::new(cmd!("shift-c"), ToggleSwitcher, Some("Workspace")),
    ]);
}
