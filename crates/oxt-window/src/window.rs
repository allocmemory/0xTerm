//! One 0xTerm window: a title row with tabs, the active tab's split panes in
//! the middle, and the toolbelt panels around them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use oxt_config::{Chrome, Config};
use oxt_session::{MAX_COPY, SessionOptions};
use oxt_task::Command;
use oxt_terminal_view::{TerminalEvent, TerminalStyle, TerminalView};
use oxt_toolbelt::{Sidebar, ToolbeltEvent, Viewer, shell_quote};

use crate::notices;
use crate::pane::{Pane, PaneId, render_pane};
use crate::split::{Axis, Node};
use crate::tab_bar::render_title_row;

actions!(
    workspace,
    [
        NewTab,
        ClosePane,
        CloseTab,
        SplitRight,
        SplitDown,
        NextPane,
        PreviousPane,
        NextTab,
        PreviousTab,
        ToggleSidebar,
        ToggleViewer,
        SearchFolder,
        FindInFile
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-t", NewTab, None),
        KeyBinding::new("cmd-w", ClosePane, None),
        KeyBinding::new("cmd-shift-w", CloseTab, None),
        KeyBinding::new("cmd-d", SplitRight, None),
        KeyBinding::new("cmd-shift-d", SplitDown, None),
        KeyBinding::new("cmd-]", NextPane, None),
        KeyBinding::new("cmd-[", PreviousPane, None),
        KeyBinding::new("cmd-}", NextTab, None),
        KeyBinding::new("cmd-{", PreviousTab, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("cmd-shift-[", PreviousTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-b", ToggleViewer, None),
        KeyBinding::new("cmd-shift-f", SearchFolder, None),
        KeyBinding::new("cmd-f", FindInFile, None),
    ]);
}

pub(crate) struct Tab {
    pub root: Node<PaneId>,
    pub panes: HashMap<PaneId, Pane>,
    pub active: PaneId,
}

impl Tab {
    pub fn label(&self) -> SharedString {
        self.panes.get(&self.active).map(|p| p.label.clone()).unwrap_or_default()
    }
}

/// What the mouse is dragging right now.
enum Drag {
    Divider { path: Vec<bool>, axis: Axis },
    Sidebar,
    Viewer,
}

/// Identifies the clipboard notice, so a new one replaces the last.
struct ClipboardNotice;

pub struct Workspace {
    config: Config,
    pub(crate) chrome: Chrome,
    style: TerminalStyle,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    next_id: PaneId,
    sidebar: Entity<Sidebar>,
    /// The folder the tree last followed; it follows the active pane's shell.
    followed: Option<PathBuf>,
    viewer: Entity<Viewer>,
    pub(crate) sidebar_open: bool,
    pub(crate) viewer_open: bool,
    sidebar_width: Pixels,
    viewer_width: Pixels,
    drag: Option<Drag>,
    /// Bounds of each split container in the active tab, by path from the root;
    /// recorded every frame so a divider drag knows what it's dividing.
    split_bounds: Rc<RefCell<HashMap<Vec<bool>, Bounds<Pixels>>>>,
    body_bounds: Rc<RefCell<Bounds<Pixels>>>,
    error: Option<String>,
}

impl Workspace {
    pub fn new(config: Config, cwd: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dark = config.is_dark(is_dark(window));
        let chrome = Chrome::for_appearance(dark);
        let start = cwd.clone().or_else(|| std::env::var_os("HOME").map(PathBuf::from)).unwrap_or_else(|| "/".into());
        let sidebar = cx.new(|cx| Sidebar::new(start, chrome, window, cx));
        let viewer = cx.new(|cx| Viewer::new(chrome, dark, window, cx));
        set_component_theme(dark, window, cx);
        cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| this.on_toolbelt_event(event, window, cx)).detach();
        let mut workspace = Workspace {
            sidebar,
            followed: None,
            viewer,
            chrome,
            style: TerminalStyle::from_config(&config, dark),
            config,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            sidebar_open: true,
            viewer_open: false,
            sidebar_width: px(248.),
            viewer_width: px(520.),
            drag: None,
            split_bounds: Default::default(),
            body_bounds: Default::default(),
            error: None,
        };
        workspace.new_tab_in(cwd, window, cx);

        cx.observe_window_appearance(window, |this, window, cx| this.appearance_changed(window, cx)).detach();

        // Labels follow the shell: its directory, or the program in front. Cheap syscalls, once a second.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update_in(cx, |this, window, cx| this.refresh_labels(window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        workspace
    }

    /// A new config.toml: fonts, colors and theme apply to every pane right away.
    pub fn set_config(&mut self, config: Config, window: &mut Window, cx: &mut Context<Self>) {
        if config != self.config {
            self.config = config;
            self.appearance_changed(window, cx);
        }
    }

    fn appearance_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dark = self.config.is_dark(is_dark(window));
        self.chrome = Chrome::for_appearance(dark);
        self.style = TerminalStyle::from_config(&self.config, dark);
        let chrome = self.chrome;
        self.sidebar.update(cx, |sidebar, cx| sidebar.set_chrome(chrome, cx));
        self.viewer.update(cx, |viewer, cx| viewer.set_theme(chrome, dark, cx));
        set_component_theme(dark, window, cx);
        for tab in &self.tabs {
            for pane in tab.panes.values() {
                let style = self.style.clone();
                pane.view.update(cx, |view, cx| view.set_style(style, cx));
            }
        }
        cx.notify();
    }

    fn spawn_pane(&mut self, cwd: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Option<(PaneId, Pane)> {
        let mut command =
            if self.config.terminal.shell.is_empty() { Command::login_shell() } else { Command::new(&self.config.terminal.shell).login() };
        if let Some(dir) = cwd.or_else(|| std::env::var_os("HOME").map(PathBuf::from)) {
            command = command.cwd(dir);
        }
        let options = SessionOptions {
            command,
            scrollback: self.config.terminal.scrollback,
            option_as_meta: self.config.terminal.option_as_meta,
            ..SessionOptions::default()
        };
        let view = match TerminalView::spawn(options, self.style.clone(), window, cx) {
            Ok(view) => view,
            Err(err) => {
                self.error = Some(format!("Couldn't start the shell: {err}"));
                return None;
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        let events = cx.subscribe_in(&view, window, move |this, _, event, window, cx| this.on_terminal_event(id, event, window, cx));
        let label = Pane::compute_label(&view, cx);
        Some((id, Pane { view, label, _events: events }))
    }

    fn on_terminal_event(&mut self, id: PaneId, event: &TerminalEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::Exited => self.close_pane(id, window, cx),
            TerminalEvent::Focused => {
                if let Some(t) = self.tabs.iter().position(|t| t.panes.contains_key(&id)) {
                    self.tabs[t].active = id;
                    self.follow_cwd(cx);
                    cx.notify();
                }
            }
            TerminalEvent::TitleChanged | TerminalEvent::CwdChanged(_) => self.refresh_labels(window, cx),
            TerminalEvent::Bell => {}
            // One notice at a time: a later copy replaces it instead of stacking (vim and tmux yanks).
            TerminalEvent::Copied(chars) => window.push_notification(Notification::info(notices::copied(*chars)).id::<ClipboardNotice>(), cx),
            TerminalEvent::CopyBlocked(bytes) => {
                window.push_notification(Notification::warning(notices::copy_blocked(*bytes, MAX_COPY)).id::<ClipboardNotice>(), cx)
            }
            TerminalEvent::OpenFile { path, line } => {
                let request = oxt_toolbelt::OpenRequest { path: path.clone(), line: *line, find: None };
                self.open(request, window, cx);
            }
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    fn active_pane(&self) -> Option<&Pane> {
        self.active_tab().and_then(|t| t.panes.get(&t.active))
    }

    /// The active pane's directory: new tabs and splits start there.
    pub fn active_cwd(&self, cx: &App) -> Option<PathBuf> {
        self.active_pane().and_then(|p| p.view.read(cx).session().cwd())
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(pane) = self.active_pane() {
            window.focus(&pane.view.focus_handle(cx), cx);
        }
    }

    /// Types text into the active pane as if the user had.
    pub fn type_into_active(&self, text: &str, cx: &App) {
        if let Some(pane) = self.active_pane() {
            pane.view.read(cx).session().type_text(text);
        }
    }

    fn new_tab_in(&mut self, cwd: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, pane)) = self.spawn_pane(cwd, window, cx) {
            let tab = Tab { root: Node::Leaf(id), panes: HashMap::from([(id, pane)]), active: id };
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.active_cwd(cx);
        self.new_tab_in(cwd, window, cx);
    }

    pub(crate) fn dispatch_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab(&NewTab, window, cx);
    }

    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.active_cwd(cx);
        let Some((id, pane)) = self.spawn_pane(cwd, window, cx) else { return };
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.root.split(tab.active, axis, id);
        tab.panes.insert(id, pane);
        tab.active = id;
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn close_pane(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter().position(|t| t.panes.contains_key(&id)) else { return };
        let tab = &mut self.tabs[t];
        if tab.root.remove(id) {
            // Dropping the pane drops its session, which hangs up the shell.
            tab.panes.remove(&id);
            if tab.active == id {
                tab.active = tab.root.panes()[0];
            }
            if t == self.active {
                self.focus_active(window, cx);
            }
            cx.notify();
        } else {
            self.close_tab(t, window, cx);
        }
    }

    pub(crate) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            window.remove_window();
            return;
        }
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn activate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.active = index;
            self.focus_active(window, cx);
            self.follow_cwd(cx);
            cx.notify();
        }
    }

    fn cycle_pane(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.active = tab.root.cycle(tab.active, forward);
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        if n > 1 {
            let next = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
            self.activate_tab(next, window, cx);
        }
    }

    pub(crate) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn toggle_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer_open = !self.viewer_open;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Points the tree at the active pane's directory when the shell `cd`s.
    fn follow_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self.active_cwd(cx) else { return };
        if self.followed.as_ref() != Some(&cwd) {
            self.followed = Some(cwd.clone());
            self.sidebar.update(cx, |sidebar, cx| sidebar.set_root(cwd, cx));
        }
    }

    fn on_toolbelt_event(&mut self, event: &ToolbeltEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ToolbeltEvent::Open(request) => {
                self.viewer.update(cx, |viewer, cx| viewer.open(request.clone(), window, cx));
                self.viewer_open = true;
                cx.notify();
            }
            ToolbeltEvent::InsertPath(path) => {
                if let Some(pane) = self.active_pane() {
                    let text = format!("{} ", shell_quote(path, pane.shell(cx)));
                    pane.view.update(cx, |view, cx| view.insert_text(&text, cx));
                }
                self.focus_active(window, cx);
            }
            ToolbeltEvent::CdHere(dir) => {
                if let Some(pane) = self.active_pane() {
                    let shell = pane.shell(cx);
                    let view = pane.view.read(cx);
                    // Only an idle shell gets a command typed into it.
                    if !view.session().is_busy() {
                        // Pasted, then Enter on its own: inside a paste it wouldn't run.
                        view.session().paste(&format!("cd {}", shell_quote(dir, shell)));
                        view.session().write(*b"\r");
                    }
                }
                self.focus_active(window, cx);
            }
            ToolbeltEvent::NewTabHere(dir) => self.new_tab_in(Some(dir.clone()), window, cx),
        }
    }

    /// Opens a file in the viewer, as if clicked in the tree.
    pub fn open(&mut self, request: oxt_toolbelt::OpenRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.on_toolbelt_event(&ToolbeltEvent::Open(request), window, cx);
    }

    /// Types a dropped path into a pane.
    pub(crate) fn drop_path(&mut self, id: PaneId, path: &std::path::Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.panes.contains_key(&id)) else { return };
        tab.active = id;
        let text = format!("{} ", shell_quote(path, tab.panes[&id].shell(cx)));
        tab.panes[&id].view.update(cx, |view, cx| view.insert_text(&text, cx));
        self.focus_active(window, cx);
    }

    fn refresh_labels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_cwd(cx);
        let mut changed = false;
        for tab in &mut self.tabs {
            for pane in tab.panes.values_mut() {
                let label = Pane::compute_label(&pane.view, cx);
                if label != pane.label {
                    pane.label = label;
                    changed = true;
                }
            }
        }
        if changed {
            let title = self.active_tab().map(|t| t.label()).unwrap_or_default();
            window.set_window_title(&title);
            cx.notify();
        }
    }

    pub(crate) fn start_drag_sidebar(&mut self) {
        self.drag = Some(Drag::Sidebar);
    }

    pub(crate) fn start_drag_viewer(&mut self) {
        self.drag = Some(Drag::Viewer);
    }

    pub(crate) fn start_drag_divider(&mut self, path: Vec<bool>, axis: Axis) {
        self.drag = Some(Drag::Divider { path, axis });
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let pos = event.position;
        let body = *self.body_bounds.borrow();
        match drag {
            Drag::Sidebar => self.sidebar_width = (pos.x - body.left()).clamp(px(160.), px(480.)),
            Drag::Viewer => self.viewer_width = (body.right() - pos.x).clamp(px(240.), (body.size.width * 0.75).max(px(240.))),
            Drag::Divider { path, axis } => {
                let Some(bounds) = self.split_bounds.borrow().get(path).copied() else { return };
                let ratio = match axis {
                    Axis::Row => (pos.x - bounds.left()) / bounds.size.width,
                    Axis::Column => (pos.y - bounds.top()) / bounds.size.height,
                };
                let path = path.clone();
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.root.set_ratio(&path, ratio);
                }
            }
        }
        cx.notify();
    }

    fn render_node(&self, tab: &Tab, node: &Node<PaneId>, path: Vec<bool>, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Leaf(id) => match tab.panes.get(id) {
                Some(pane) => render_pane(pane, *id, tab, &self.chrome, cx),
                None => div().into_any_element(),
            },
            Node::Split { axis, ratio, a, b } => {
                let record = self.split_bounds.clone();
                let key = path.clone();
                let measure = canvas(
                    move |bounds, _, _| {
                        record.borrow_mut().insert(key, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full();
                let first = div().relative().flex().min_w_0().min_h_0().overflow_hidden();
                let first = match axis {
                    Axis::Row => first.h_full().w(relative(*ratio)),
                    Axis::Column => first.w_full().h(relative(*ratio)),
                };
                let mut a_path = path.clone();
                a_path.push(false);
                let mut b_path = path.clone();
                b_path.push(true);
                let container = div().relative().flex().size_full().min_w_0().min_h_0();
                let container = match axis {
                    Axis::Row => container.flex_row(),
                    Axis::Column => container.flex_col(),
                };
                container
                    .child(measure)
                    .child(first.child(self.render_node(tab, a, a_path, cx)))
                    .child(self.render_divider(*axis, path, cx))
                    .child(div().relative().flex().flex_1().min_w_0().min_h_0().overflow_hidden().child(self.render_node(tab, b, b_path, cx)))
                    .into_any_element()
            }
        }
    }

    fn render_divider(&self, axis: Axis, path: Vec<bool>, cx: &mut Context<Self>) -> impl IntoElement {
        let line = div().flex_none().relative().bg(rgba(self.chrome.border));
        let (line, grip) = match axis {
            Axis::Row => (line.w(px(1.)).h_full(), div().absolute().top_0().bottom_0().left(px(-3.)).w(px(7.)).cursor_col_resize()),
            Axis::Column => (line.h(px(1.)).w_full(), div().absolute().left_0().right_0().top(px(-3.)).h(px(7.)).cursor_row_resize()),
        };
        line.child(grip.id(SharedString::from(format!("divider-{path:?}"))).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.start_drag_divider(path.clone(), axis);
                cx.stop_propagation();
            }),
        ))
    }

    fn render_resizer(&self, which: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex_none().w(px(1.)).h_full().relative().bg(rgba(self.chrome.border)).child(
            div().id(which).absolute().top_0().bottom_0().left(px(-3.)).w(px(7.)).cursor_col_resize().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if which == "sidebar-resizer" {
                        this.start_drag_sidebar();
                    } else {
                        this.start_drag_viewer();
                    }
                    cx.stop_propagation();
                }),
            ),
        )
    }
}

/// gpui-component widgets (inputs, editor, Markdown) follow light/dark too.
fn set_component_theme(dark: bool, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::theme::{Theme, ThemeMode};
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, Some(window), cx);
}

pub fn is_dark(window: &Window) -> bool {
    matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = self.chrome;
        let term_bg = crate::pane::rgb_hsla(self.style.palette.background);
        let body_record = self.body_bounds.clone();

        let panes = match (self.active_tab(), &self.error) {
            (Some(tab), _) => self.render_node(tab, &tab.root, Vec::new(), cx),
            (None, Some(error)) => div().p_4().text_color(rgb(0xff6b6b)).child(error.clone()).into_any_element(),
            (None, None) => div().into_any_element(),
        };

        let mut body = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .relative()
            .child(canvas(move |bounds, _, _| *body_record.borrow_mut() = bounds, |_, _, _, _| {}).absolute().size_full());
        if self.sidebar_open {
            body = body
                .child(div().flex_none().w(self.sidebar_width).h_full().bg(rgba(chrome.panel)).child(self.sidebar.clone()))
                .child(self.render_resizer("sidebar-resizer", cx));
        }
        body = body.child(div().flex_1().min_w_0().h_full().relative().overflow_hidden().bg(term_bg).child(panes));
        if self.viewer_open {
            body = body
                .child(self.render_resizer("viewer-resizer", cx))
                .child(div().flex_none().w(self.viewer_width).h_full().bg(term_bg).child(self.viewer.clone()));
        }

        div()
            .key_context("Workspace")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba(chrome.chrome))
            .text_color(rgba(chrome.text))
            .text_size(px(13.))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(|this, _: &ClosePane, window, cx| {
                if let Some(tab) = this.active_tab() {
                    let id = tab.active;
                    this.close_pane(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close_tab(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| this.split(Axis::Row, window, cx)))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| this.split(Axis::Column, window, cx)))
            .on_action(cx.listener(|this, _: &NextPane, window, cx| this.cycle_pane(true, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousPane, window, cx| this.cycle_pane(false, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle_tab(true, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.cycle_tab(false, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleViewer, window, cx| this.toggle_viewer(window, cx)))
            .on_action(cx.listener(|this, _: &FindInFile, window, cx| {
                if !this.viewer.read(cx).is_empty() {
                    this.viewer_open = true;
                    this.viewer.update(cx, |viewer, cx| viewer.focus_find(window, cx));
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SearchFolder, window, cx| {
                this.sidebar_open = true;
                this.sidebar.update(cx, |sidebar, cx| sidebar.show_search(window, cx));
                cx.notify();
            }))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.drag = None))
            .child(render_title_row(self, cx))
            .child(body)
    }
}
