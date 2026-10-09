//! The native menu bar. Every action is reachable from here, and macOS shows
//! each item's shortcut next to it.

use gpui_kit::*;
use oxt_terminal_view::{Clear, Copy, Paste, SelectAll};
use oxt_window::{ClosePane, CloseTab, NewTab, NextPane, NextTab, PreviousPane, PreviousTab, SplitDown, SplitRight, ToggleSidebar, ToggleViewer};

use crate::{NewWindow, Quit};

pub fn install(cx: &mut App) {
    cx.set_menus(vec![
        Menu { name: "0xTerm".into(), items: vec![MenuItem::action("Quit 0xTerm", Quit)], disabled: false },
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
            items: vec![MenuItem::action("Toggle Files", ToggleSidebar), MenuItem::action("Toggle Viewer", ToggleViewer)],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Next Pane", NextPane),
                MenuItem::action("Previous Pane", PreviousPane),
                MenuItem::separator(),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PreviousTab),
            ],
            disabled: false,
        },
    ]);
}
