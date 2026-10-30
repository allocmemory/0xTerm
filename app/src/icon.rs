//! `[app] icon`: shows the chosen icon at launch and whenever config.toml
//! changes it. A choice that can't be found or loaded shows the built-in icon.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use oxt_config::{Config, IconChoice};

use crate::macos::{self, IconApplied};

thread_local! {
    /// The `icon` value last applied, so a reload that changed something else
    /// doesn't touch the bundle.
    static APPLIED: RefCell<Option<String>> = const { RefCell::new(None) };
    /// What the last apply managed, for Settings to explain.
    static RESULT: Cell<Option<IconApplied>> = const { Cell::new(None) };
}

/// What the last change of icon managed to do, if one happened this run.
pub fn last_applied() -> Option<IconApplied> {
    RESULT.with(Cell::get)
}

/// The file a choice points at, if it exists.
pub fn path_for(choice: &IconChoice) -> Option<PathBuf> {
    let path = match choice {
        IconChoice::Default => return None,
        IconChoice::Bundled(name) => macos::bundled_icons_dir()?.join(format!("{name}.icns")),
        IconChoice::Imported(file) => Config::icons_dir()?.join(file),
    };
    path.is_file().then_some(path)
}

/// At startup, before the first window. The Dock image only lasts while the
/// app runs, so it's set every launch; the Finder icon is rewritten so a hand
/// edit made while 0xTerm was closed, or an update that replaced the bundle,
/// is picked up.
pub fn apply_at_launch(config: &Config) {
    let choice = IconChoice::parse(&config.app.icon);
    if choice == IconChoice::Default && !macos::app_has_custom_icon() {
        remember(config);
        return;
    }
    apply(config);
}

/// After a config reload: only when the icon value changed.
pub fn apply_if_changed(config: &Config) {
    if APPLIED.with(|applied| applied.borrow().as_deref() != Some(config.app.icon.as_str())) {
        apply(config);
    }
}

/// Applies the configured icon and says what happened, for Settings to show.
pub fn apply(config: &Config) -> IconApplied {
    remember(config);
    let choice = IconChoice::parse(&config.app.icon);
    let path = path_for(&choice);
    if path.is_none() && choice != IconChoice::Default {
        eprintln!("0xTerm: icon {:?} not found, showing the built-in icon", config.app.icon);
    }
    let applied = macos::set_app_icon(path.as_deref());
    if applied == IconApplied::Unloadable {
        eprintln!("0xTerm: icon {:?} couldn't be loaded, showing the built-in icon", config.app.icon);
        macos::set_app_icon(None);
    }
    RESULT.with(|result| result.set(Some(applied)));
    applied
}

fn remember(config: &Config) {
    APPLIED.with(|applied| *applied.borrow_mut() = Some(config.app.icon.clone()));
}
