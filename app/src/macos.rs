//! The two pieces of AppKit that GPUI doesn't wrap: Sparkle's updater and the
//! standard About panel.
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
