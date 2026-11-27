//! `[app] icon`: shows the chosen icon at launch, whenever config.toml
//! changes it, and whenever the macOS appearance switches between light and
//! dark (official icons come in both). A choice that can't be found or loaded
//! shows the built-in icon.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use oxt_config::icon::Appearance;
use oxt_config::{Config, IconChoice};

use crate::macos::{self, IconApplied};

thread_local! {
    /// The `icon` value and appearance last applied, so a reload that changed
    /// something else, or an appearance notice that changed nothing, doesn't
    /// touch the bundle.
    static APPLIED: RefCell<Option<(String, Appearance)>> = const { RefCell::new(None) };
    /// What the last apply managed, for Settings to explain.
    static RESULT: Cell<Option<IconApplied>> = const { Cell::new(None) };
}

/// What the last change of icon managed to do, if one happened this run.
pub fn last_applied() -> Option<IconApplied> {
    RESULT.with(Cell::get)
}

/// The file a choice shows in `appearance`, if it exists. `None` for the
/// built-in icon in dark mode, which is the one compiled into the app.
pub fn path_for(choice: &IconChoice, appearance: Appearance) -> Option<PathBuf> {
    let path = match choice {
        IconChoice::Imported(file) => Config::icons_dir()?.join(file),
        _ => macos::bundled_icons_dir()?.join(format!("{}.icns", choice.bundled_stem(appearance)?)),
    };
    path.is_file().then_some(path)
}

/// At startup, before the first window. The Dock image only lasts while the
/// app runs, so it's set every launch; the Finder icon is rewritten so a hand
/// edit made while 0xTerm was closed, an appearance switch, or an update that
/// replaced the bundle is picked up. Then follows the appearance from here on.
pub fn apply_at_launch(config: &Config) {
    macos::observe_appearance(appearance_changed);
    let appearance = macos::appearance();
    let choice = IconChoice::parse(&config.app.icon);
    if choice == IconChoice::Default && appearance == Appearance::Dark && !macos::app_has_custom_icon() {
        remember(&config.app.icon, appearance);
        return;
    }
    apply(config);
}

/// After a config reload: only when the icon value changed.
pub fn apply_if_changed(config: &Config) {
    if APPLIED.with(|applied| applied.borrow().as_ref().map(|(value, _)| value.as_str()) != Some(config.app.icon.as_str())) {
        apply(config);
    }
}

/// Applies the configured icon and says what happened, for Settings to show.
pub fn apply(config: &Config) -> IconApplied {
    apply_value(&config.app.icon, macos::appearance())
}

/// The macOS appearance may have changed: show the other variant of the
/// current icon, and let an open Settings window redraw its tiles.
fn appearance_changed() {
    let appearance = macos::appearance();
    let Some((value, applied)) = APPLIED.with(|applied| applied.borrow().clone()) else { return };
    if applied == appearance {
        return;
    }
    // An imported icon is one file whatever the appearance: nothing to redraw.
    if matches!(IconChoice::parse(&value), IconChoice::Imported(_)) {
        remember(&value, appearance);
    } else {
        apply_value(&value, appearance);
    }
    crate::settings_window::refresh();
}

fn apply_value(value: &str, appearance: Appearance) -> IconApplied {
    remember(value, appearance);
    let choice = IconChoice::parse(value);
    let path = path_for(&choice, appearance);
    if path.is_none() && choice != IconChoice::Default {
        eprintln!("0xTerm: icon {value:?} not found, showing the built-in icon");
    }
    let applied = macos::set_app_icon(path.as_deref());
    if applied == IconApplied::Unloadable {
        eprintln!("0xTerm: icon {value:?} couldn't be loaded, showing the built-in icon");
        macos::set_app_icon(None);
    }
    RESULT.with(|result| result.set(Some(applied)));
    applied
}

fn remember(value: &str, appearance: Appearance) {
    APPLIED.with(|applied| *applied.borrow_mut() = Some((value.to_string(), appearance)));
}
