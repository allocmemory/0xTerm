//! The pieces of AppKit that GPUI doesn't wrap: Sparkle's updater, the
//! standard About panel and the app icon.
//!
//! `packaging/bundle.sh` embeds Sparkle.framework in Contents/Frameworks and
//! this loads it at startup, so a `cargo run` build (no bundle, no framework)
//! simply runs without updates.

use std::cell::OnceCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{class, msg_send};
use objc2_foundation::{NSBundle, NSString};

thread_local! {
    /// Sparkle's controller. Created once on the main thread and kept for the
    /// life of the app: dropping it stops the scheduled update checks.
    static UPDATER: OnceCell<Retained<AnyObject>> = const { OnceCell::new() };
}

/// Loads Sparkle and starts its updater, which checks on the schedule the
/// user picks the first time it asks. Does nothing outside an app bundle.
pub fn start_updater() {
    let Some(class) = load_sparkle() else { return };
    // SAFETY: SPUStandardUpdaterController's designated initializer; nil
    // delegates mean Sparkle's standard behavior and UI.
    let controller: Option<Retained<AnyObject>> = unsafe {
        msg_send![msg_send![class, alloc], initWithStartingUpdater: true, updaterDelegate: None::<&AnyObject>, userDriverDelegate: None::<&AnyObject>]
    };
    if let Some(controller) = controller {
        UPDATER.with(|cell| _ = cell.set(controller));
    }
}

pub fn updater_available() -> bool {
    UPDATER.with(|cell| cell.get().is_some())
}

/// Checks now and shows the result, as "Check for Updates…" does in any Mac app.
pub fn check_for_updates() {
    UPDATER.with(|cell| {
        if let Some(controller) = cell.get() {
            // SAFETY: -[SPUStandardUpdaterController checkForUpdates:] takes an ignored sender.
            unsafe {
                let _: () = msg_send![&**controller, checkForUpdates: None::<&AnyObject>];
            }
        }
    });
}

/// The standard About panel: icon, name, version and copyright from Info.plist.
pub fn show_about_panel() {
    // SAFETY: called on the main thread from a menu action.
    unsafe {
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![&*app, orderFrontStandardAboutPanel: None::<&AnyObject>];
    }
}

/// What [`set_app_icon`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconApplied {
    /// The Dock shows it now, and Finder, Launchpad and the Dock keep it after quit.
    Persisted,
    /// The Dock shows it while 0xTerm runs, but 0xTerm.app couldn't be marked
    /// (not writable, or not running from an app bundle).
    RuntimeOnly,
    /// AppKit couldn't load the file; nothing changed.
    Unloadable,
}

/// Shows `path` as the app icon, or the built-in one with `None`: in the Dock
/// right away, and as 0xTerm.app's Finder icon so it stays after quit. A
/// custom Finder icon is a file inside the bundle, which `codesign --strict`
/// reports but Gatekeeper and Sparkle accept.
pub fn set_app_icon(path: Option<&std::path::Path>) -> IconApplied {
    // SAFETY: AppKit classes used as documented, on the main thread (startup
    // and config reload both run there).
    unsafe {
        let image: Option<Retained<AnyObject>> = match path {
            Some(path) => {
                let path = NSString::from_str(&path.to_string_lossy());
                let image: Option<Retained<AnyObject>> = msg_send![msg_send![class!(NSImage), alloc], initWithContentsOfFile: &*path];
                if image.is_none() {
                    return IconApplied::Unloadable;
                }
                image
            }
            None => None,
        };
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![&*app, setApplicationIconImage: image.as_deref()];
        let Some(bundle) = app_bundle_path() else { return IconApplied::RuntimeOnly };
        let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
        let bundle = NSString::from_str(&bundle.to_string_lossy());
        let ok: bool = msg_send![&*workspace, setIcon: image.as_deref(), forFile: &*bundle, options: 0usize];
        if ok { IconApplied::Persisted } else { IconApplied::RuntimeOnly }
    }
}

/// Whether 0xTerm.app carries a custom Finder icon. An update replaces the
/// bundle, and with it the icon.
pub fn app_has_custom_icon() -> bool {
    app_bundle_path().is_some_and(|bundle| bundle.join("Icon\r").exists())
}

/// The official icon set inside the bundle (`packaging/bundle.sh` puts it there).
pub fn bundled_icons_dir() -> Option<std::path::PathBuf> {
    let resources = NSBundle::mainBundle().resourcePath()?;
    Some(std::path::PathBuf::from(resources.to_string()).join("Icons"))
}

/// 0xTerm.app, or `None` for a `cargo run` build, which has no bundle to mark.
fn app_bundle_path() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(NSBundle::mainBundle().bundlePath().to_string());
    (path.extension().is_some_and(|ext| ext == "app")).then_some(path)
}

fn load_sparkle() -> Option<&'static AnyClass> {
    let frameworks = NSBundle::mainBundle().privateFrameworksPath()?;
    let path = NSString::from_str(&format!("{frameworks}/Sparkle.framework"));
    let bundle = NSBundle::bundleWithPath(&path)?;
    // SAFETY: loads code from our own signed bundle; under the hardened
    // runtime macOS only allows it if it carries our Team ID.
    if !unsafe { bundle.load() } {
        return None;
    }
    AnyClass::get(c"SPUStandardUpdaterController")
}
