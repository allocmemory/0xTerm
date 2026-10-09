//! The terminal pane: the grid drawn on the GPU through GPUI, plus selection, links and input.

mod element;
pub mod links;
pub mod style;
mod view;

pub use style::TerminalStyle;
pub use view::{Clear, Copy, KEY_CONTEXT, Paste, SelectAll, TerminalEvent, TerminalView, bind_keys};
