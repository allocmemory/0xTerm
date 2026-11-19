//! The file viewer: code with find, Markdown with Mermaid, images. Text
//! tabs are editable and save with ⌘S.

pub mod find;
pub mod load;
pub mod markdown;
pub mod mermaid;
pub mod save;
mod view;

pub use view::{Save, Viewer};
