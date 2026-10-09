//! The toolbelt: the panels beside the panes. File tree, folder search and
//! the file viewer.

pub mod events;
pub mod file_tree;
pub mod search;
mod search_view;
mod sidebar;
mod tree_view;
pub mod viewer;

pub use events::{DraggedPath, OpenRequest, ToolbeltEvent, shell_quote};
pub use sidebar::{Sidebar, SidebarMode};
pub use viewer::Viewer;
