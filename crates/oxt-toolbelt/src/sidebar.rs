//! The left panel: Files | Search, with breadcrumbs over the tree.

use std::path::PathBuf;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use oxt_config::Chrome;

use crate::events::ToolbeltEvent;
use crate::file_tree;
use crate::search_view::SearchPanel;
use crate::tree_view::FileTree;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    Files,
    Search,
}

pub struct Sidebar {
    mode: SidebarMode,
    tree: Entity<FileTree>,
    search: Entity<SearchPanel>,
    chrome: Chrome,
    home: Option<PathBuf>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ToolbeltEvent> for Sidebar {}

impl Sidebar {
    pub fn new(root: PathBuf, chrome: Chrome, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| FileTree::new(root.clone(), chrome, cx));
        let search = cx.new(|cx| SearchPanel::new(root, chrome, window, cx));
        let forward = |_: &mut Sidebar, event: &ToolbeltEvent, cx: &mut Context<Sidebar>| cx.emit(event.clone());
        let subscriptions = vec![
            cx.subscribe(&tree, move |this, _, event, cx| forward(this, event, cx)),
            cx.subscribe(&search, move |this, _, event, cx| forward(this, event, cx)),
        ];
        Sidebar { mode: SidebarMode::Files, tree, search, chrome, home: std::env::var_os("HOME").map(PathBuf::from), _subscriptions: subscriptions }
    }

    pub fn root(&self, cx: &App) -> PathBuf {
        self.tree.read(cx).root().to_path_buf()
    }

    /// The folder both the tree and the search look at.
    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.tree.update(cx, |tree, cx| tree.set_root(root.clone(), cx));
        self.search.update(cx, |search, cx| search.set_root(root, cx));
        cx.notify();
    }

    pub fn set_chrome(&mut self, chrome: Chrome, cx: &mut Context<Self>) {
        self.chrome = chrome;
        self.tree.update(cx, |t, cx| {
            t.chrome = chrome;
            cx.notify();
        });
        self.search.update(cx, |s, cx| {
            s.chrome = chrome;
            cx.notify();
        });
        cx.notify();
    }

    pub fn show_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = SidebarMode::Search;
        let input = self.search.read(cx).input();
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn render_crumbs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.tree.read(cx).root().to_path_buf();
        let chrome = self.chrome;
        let parts = file_tree::crumbs(&root, self.home.as_deref(), 3);
        let last = parts.len().saturating_sub(1);
        let mut row =
            div().flex().flex_row().items_center().gap(px(2.)).px(px(10.)).pb(px(6.)).text_size(px(12.)).overflow_hidden().whitespace_nowrap();
        for (i, (name, path)) in parts.into_iter().enumerate() {
            if i > 0 && name != "/" {
                row = row.child(div().text_color(rgba(chrome.faint)).child("/"));
            }
            let target = path.clone();
            row = row.child(
                div()
                    .id(("crumb", i))
                    .px(px(3.))
                    .rounded(px(4.))
                    .text_color(rgba(if i == last { chrome.text } else { chrome.dim }))
                    .hover(|s| s.bg(rgba(chrome.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_root(target.clone(), cx)))
                    .child(name),
            );
        }
        row
    }

    fn segment(&self, label: &'static str, mode: SidebarMode, cx: &mut Context<Self>) -> impl IntoElement {
        let on = self.mode == mode;
        let chrome = self.chrome;
        div()
            .id(label)
            .flex_1()
            .flex()
            .justify_center()
            .py(px(3.))
            .rounded(px(5.))
            .text_size(px(12.))
            .text_color(rgba(if on { chrome.text } else { chrome.dim }))
            .when(on, |d| d.bg(rgba(chrome.panel)).shadow_sm())
            .on_click(cx.listener(move |this, _, window, cx| {
                if mode == SidebarMode::Search {
                    this.show_search(window, cx);
                } else {
                    this.mode = mode;
                    cx.notify();
                }
            }))
            .child(label)
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = self.chrome;
        let body = match self.mode {
            SidebarMode::Files => {
                div().flex().flex_col().flex_1().min_h_0().child(self.render_crumbs(cx)).child(div().flex_1().min_h_0().child(self.tree.clone()))
            }
            SidebarMode::Search => div().flex().flex_col().flex_1().min_h_0().child(self.search.clone()),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba(chrome.panel))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .m(px(8.))
                    .p(px(2.))
                    .rounded(px(7.))
                    .bg(rgba(chrome.chrome_2))
                    .child(self.segment("Files", SidebarMode::Files, cx))
                    .child(self.segment("Search", SidebarMode::Search, cx)),
            )
            .child(body)
    }
}
