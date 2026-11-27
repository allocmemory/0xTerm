//! `[app] icon` and `icon_appearance`: shows the chosen icon at launch and
//! whenever config.toml changes it. Official icons come in a dark and a light
//! version; with `icon_appearance = "system"` the version also follows the
//! macOS appearance as it switches. A choice that can't be found or loaded
//! shows the built-in icon.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use oxt_config::icon::Appearance;
use oxt_config::{Config, IconAppearance, IconChoice};

use crate::macos::{self, IconApplied};

/// What was last applied: the `icon` value, the `icon_appearance` setting,
/// and the version that setting resolved to.
#[derive(Clone, PartialEq)]
struct Applied {
    value: String,
    mode: IconAppearance,
    shown: Appearance,
}

thread_local! {
    /// So a reload that changed something else, or an appearance notice that
    /// changed nothing, doesn't touch the bundle.
    static APPLIED: RefCell<Option<Applied>> = const { RefCell::new(None) };
    /// What the last apply managed, for Settings to explain.
    static RESULT: Cell<Option<IconApplied>> = const { Cell::new(None) };
}

/// What the last change of icon managed to do, if one happened this run.
pub fn last_applied() -> Option<IconApplied> {
    RESULT.with(Cell::get)
}

/// The version of the icons to show under this config right now.
pub fn appearance_for(config: &Config) -> Appearance {
    config.app.icon_appearance.resolve(macos::appearance())
}

/// The file a choice shows in `appearance`, if it exists. `None` for the
/// built-in icon's dark version, which is the one compiled into the app.
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
/// replaced the bundle is picked up.
pub fn apply_at_launch(config: &Config) {
    macos::observe_appearance(appearance_changed);
    let shown = appearance_for(config);
    let choice = IconChoice::parse(&config.app.icon);
    if choice == IconChoice::Default && shown == Appearance::Dark && !macos::app_has_custom_icon() {
        remember(&config.app.icon, config.app.icon_appearance, shown);
        return;
    }
    apply(config);
}

/// After a config reload: only when the icon or its version changed.
pub fn apply_if_changed(config: &Config) {
    let wanted = Applied { value: config.app.icon.clone(), mode: config.app.icon_appearance, shown: appearance_for(config) };
    if APPLIED.with(|applied| applied.borrow().as_ref() != Some(&wanted)) {
        apply(config);
    }
}

/// Applies the configured icon and says what happened, for Settings to show.
pub fn apply(config: &Config) -> IconApplied {
    apply_value(&config.app.icon, config.app.icon_appearance, appearance_for(config))
}

/// The macOS appearance may have changed. Only matters with
/// `icon_appearance = "system"`: show the other version, and let an open
/// Settings window redraw its tiles.
fn appearance_changed() {
    let Some(applied) = APPLIED.with(|applied| applied.borrow().clone()) else { return };
    if applied.mode != IconAppearance::System {
        return;
    }
    let shown = applied.mode.resolve(macos::appearance());
    if shown == applied.shown {
        return;
    }
    // An imported icon is one file whatever the appearance: nothing to redraw.
    if matches!(IconChoice::parse(&applied.value), IconChoice::Imported(_)) {
        remember(&applied.value, applied.mode, shown);
    } else {
        apply_value(&applied.value, applied.mode, shown);
    }
    crate::settings_window::refresh();
}

fn apply_value(value: &str, mode: IconAppearance, shown: Appearance) -> IconApplied {
    remember(value, mode, shown);
    let choice = IconChoice::parse(value);
    let path = path_for(&choice, shown);
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

fn remember(value: &str, mode: IconAppearance, shown: Appearance) {
    APPLIED.with(|applied| *applied.borrow_mut() = Some(Applied { value: value.to_string(), mode, shown }));
}
