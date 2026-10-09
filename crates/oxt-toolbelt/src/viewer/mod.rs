//! The file viewer: code with find, Markdown with Mermaid, images.

pub mod find;
pub mod load;
pub mod markdown;
pub mod mermaid;
mod view;

pub use view::Viewer;
