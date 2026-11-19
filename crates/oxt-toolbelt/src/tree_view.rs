//! The clickable file tree. Folders load when opened; the tree reloads a
//! folder when macOS reports a change in it (FSEvents, through `notify`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use notify::{RecursiveMode, Watcher};
use oxt_config::Chrome;

use crate::events::{DraggedPath, OpenRequest, ToolbeltEvent};
use crate::file_tree::{self, Entry};

const ROW_HEIGHT: f32 = 22.;

struct Row {
    entry: Entry,
    depth: usize,
    expanded: bool,
}

pub struct FileTree {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<Entry>>,
    rows: Vec<Row>,
    selected: Option<PathBuf>,
    pub(crate) chrome: Chrome,
    scroll: UniformListScrollHandle,
    watcher: Option<notify::RecommendedWatcher>,
    changes: async_channel::Sender<PathBuf>,
}

impl EventEmitter<ToolbeltEvent> for FileTree {}

impl FileTree {
    pub fn new(root: PathBuf, chrome: Chrome, cx: &mut Context<Self>) -> Self {
        let (changes, changed) = async_channel::unbounded::<PathBuf>();
        // Folder changes arrive in bursts (a build writes hundreds of files);
        // gather them for a moment, then reload each affected folder once.
        cx.spawn(async move |this, cx| {
            while let Ok(first) = changed.recv().await {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let mut dirs = HashSet::from([first]);
                while let Ok(more) = changed.try_recv() {
                    dirs.insert(more);
                }
                if this.update(cx, |tree, cx| tree.reload(&dirs, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut tree = FileTree {
            root: PathBuf::new(),
            expanded: HashSet::new(),
            listings: HashMap::new(),
            rows: Vec::new(),
            selected: None,
            chrome,
            scroll: UniformListScrollHandle::new(),
            watcher: None,
            changes,
        };
        tree.set_root(root, cx);
        tree
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Shows another folder. Expanded state is kept for when you come back.
    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        self.root = root;
        self.listings.clear();
        self.load(&self.root.clone());
        self.watch();
        self.rebuild();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn watch(&mut self) {
        let tx = self.changes.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    if let Some(parent) = path.parent() {
                        let _ = tx.try_send(parent.to_path_buf());
                    }
                }
            }
        });
        self.watcher = watcher.ok().and_then(|mut w| w.watch(&self.root, RecursiveMode::Recursive).ok().map(|_| w));
    }

    fn load(&mut self, dir: &Path) {
        let entries = file_tree::list(dir).unwrap_or_default();
        self.listings.insert(dir.to_path_buf(), entries);
    }

    fn reload(&mut self, dirs: &HashSet<PathBuf>, cx: &mut Context<Self>) {
        let mut changed = false;
        for dir in dirs {
            if self.listings.contains_key(dir) {
                let fresh = file_tree::list(dir).unwrap_or_default();
                if self.listings.get(dir) != Some(&fresh) {
                    self.listings.insert(dir.clone(), fresh);
                    changed = true;
                }
            }
        }
        if changed {
            self.rebuild();
            cx.notify();
        }
    }

    /// Flattens the visible part of the tree into rows for the virtual list.
    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        self.push_rows(&self.root.clone(), 0, &mut rows);
        self.rows = rows;
    }

    fn push_rows(&mut self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        if !self.listings.contains_key(dir) {
            self.load(dir);
        }
        let entries = self.listings.get(dir).cloned().unwrap_or_default();
        for entry in entries {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            let path = entry.path.clone();
            rows.push(Row { entry, depth, expanded });
            if expanded {
                self.push_rows(&path, depth + 1, rows);
            }
        }
    }

    fn click(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else { return };
        let path = row.entry.path.clone();
        self.selected = Some(path.clone());
        if row.entry.is_dir {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
            self.rebuild();
        } else {
            cx.emit(ToolbeltEvent::Open(OpenRequest::path(path)));
        }
        cx.notify();
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = &self.rows[index];
        let chrome = self.chrome;
        let selected = self.selected.as_ref() == Some(&row.entry.path);
        let icon_path = match (row.entry.is_dir, row.expanded) {
            (true, true) => "icons/folder-open.svg",
            (true, false) => "icons/folder.svg",
            _ => "icons/file.svg",
        };
        let chevron = if row.expanded { "icons/chevron-down.svg" } else { "icons/chevron-right.svg" };
        let text = if row.entry.ignored { chrome.faint } else { chrome.text };
        let path = row.entry.path.clone();
        let menu_path = path.clone();
        let tree = cx.entity();

        div()
            .id(("row", index))
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .pl(px(8. + row.depth as f32 * 14.))
            .pr(px(8.))
            .text_size(px(13.))
            .text_color(rgba(text))
            .whitespace_nowrap()
            .overflow_hidden()
            .when(selected, |d| d.bg(rgba(chrome.selected)))
            .when(!selected, |d| d.hover(|s| s.bg(rgba(chrome.hover))))
            .on_click(cx.listener(move |tree, _, _, cx| tree.click(index, cx)))
            .on_drag(DraggedPath(path.clone()), |dragged, _, _, cx| cx.new(|_| DragLabel(dragged.0.clone())))
            .child(
                div().flex_none().size(px(12.)).when(row.entry.is_dir, |d| d.child(svg().path(chevron).size(px(12.)).text_color(rgba(chrome.faint)))),
            )
            .child(svg().path(icon_path).size(px(14.)).flex_none().text_color(rgba(if row.entry.is_dir { chrome.accent } else { chrome.dim })))
            .child(div().min_w_0().overflow_hidden().text_ellipsis().child(row.entry.name.clone()))
            .context_menu(move |menu, _, _| {
                let item = |label: &'static str, event: fn(PathBuf) -> ToolbeltEvent| {
                    let (tree, path) = (tree.clone(), menu_path.clone());
                    PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        let event = event(path.clone());
                        tree.update(cx, |_, cx| cx.emit(event));
                    })
                };
                let dir = if menu_path.is_dir() { menu_path.clone() } else { menu_path.parent().map(Path::to_path_buf).unwrap_or_default() };
                let (tree_for_cd, tree_for_tab) = (tree.clone(), tree.clone());
                let (dir_for_cd, dir_for_tab) = (dir.clone(), dir);
                let copy_path = menu_path.clone();
                menu.item(PopupMenuItem::new("cd Here").on_click(move |_, _, cx| {
                    let event = ToolbeltEvent::CdHere(dir_for_cd.clone());
                    tree_for_cd.update(cx, |_, cx| cx.emit(event));
                }))
                .item(PopupMenuItem::new("New Tab Here").on_click(move |_, _, cx| {
                    let event = ToolbeltEvent::NewTabHere(dir_for_tab.clone());
                    tree_for_tab.update(cx, |_, cx| cx.emit(event));
                }))
                .separator()
                .item(item("Insert Path", ToolbeltEvent::InsertPath))
                .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_path.to_string_lossy().into_owned()));
                }))
            })
            .into_any_element()
    }
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.rows.len();
        uniform_list(
            "file-tree",
            count,
            cx.processor(|tree, range: std::ops::Range<usize>, _, cx| range.map(|i| tree.render_row(i, cx)).collect::<Vec<_>>()),
        )
        .track_scroll(&self.scroll)
        .size_full()
        .py(px(4.))
    }
}

/// What follows the mouse while a path is dragged.
pub struct DragLabel(PathBuf);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let name = self.0.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        div().px(px(8.)).py(px(3.)).rounded(px(5.)).bg(rgba(0x4c9dffcc)).text_color(rgb(0xffffff)).text_size(px(12.)).child(name)
    }
}
