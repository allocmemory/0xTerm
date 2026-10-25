//! The window: tabs, each tab's split tree of panes, and the toolbelt panels
//! around them.

mod notices;
mod pane;
pub mod split;
mod tab_bar;
mod window;

pub use window::{
    ClosePane, CloseTab, FindInFile, NewTab, NextPane, NextTab, PreviousPane, PreviousTab, SearchFolder, SplitDown, SplitRight, ToggleSidebar,
    ToggleViewer, Workspace, bind_keys, is_dark,
};
