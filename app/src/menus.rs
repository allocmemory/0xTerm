//! The native menu bar. Every action is reachable from here, and macOS shows
//! each item's shortcut next to it.

use gpui_kit::*;
use oxt_terminal_view::{Clear, Copy, Paste, SelectAll};
use oxt_window::{ClosePane, CloseTab, NewTab, NextPane, NextTab, PreviousPane, PreviousTab, SplitDown, SplitRight, ToggleSidebar, ToggleViewer};

use crate::{NewWindow, OpenSettings, Quit, macos};

actions!(oxterm, [About, CheckForUpdates, Hide, HideOthers, ShowAll, Minimize, Zoom, ToggleFullScreen, OpenHelp, ReportIssue]);

pub fn install(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);
    cx.on_action(|_: &About, _| macos::show_about_panel());
    cx.on_action(|_: &CheckForUpdates, _| macos::check_for_updates());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| with_active_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| with_active_window(cx, |window| window.zoom_window()));
    cx.on_action(|_: &ToggleFullScreen, cx| with_active_window(cx, |window| window.toggle_fullscreen()));
    cx.on_action(|_: &OpenHelp, cx| cx.open_url("https://0xterm.org/docs/"));
    cx.on_action(|_: &ReportIssue, cx| cx.open_url("https://github.com/allocmemory/0xTerm/issues"));

    cx.set_menus(vec![
        Menu {
            name: "0xTerm".into(),
            items: vec![
                MenuItem::action("About 0xTerm", About),
                MenuItem::action("Check for Updates…", CheckForUpdates).disabled(!macos::updater_available()),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide 0xTerm", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit 0xTerm", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "Shell".into(),
            items: vec![
                MenuItem::action("New Window", NewWindow),
                MenuItem::action("New Tab", NewTab),
                MenuItem::separator(),
                MenuItem::action("Split Right", SplitRight),
                MenuItem::action("Split Down", SplitDown),
                MenuItem::separator(),
                MenuItem::action("Close Pane", ClosePane),
                MenuItem::action("Close Tab", CloseTab),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Clear Buffer", Clear),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Toggle Files", ToggleSidebar),
                MenuItem::action("Toggle Viewer", ToggleViewer),
                MenuItem::separator(),
                MenuItem::action("Toggle Full Screen", ToggleFullScreen),
            ],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
                MenuItem::separator(),
                MenuItem::action("Next Pane", NextPane),
                MenuItem::action("Previous Pane", PreviousPane),
                MenuItem::separator(),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PreviousTab),
            ],
            disabled: false,
        },
        Menu {
            name: "Help".into(),
            items: vec![MenuItem::action("0xTerm Help", OpenHelp), MenuItem::action("Report an Issue", ReportIssue)],
            disabled: false,
        },
    ]);
}

fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window)) {
    if let Some(handle) = cx.active_window() {
        let _ = handle.update(cx, |_, window, _| f(window));
    }
}
