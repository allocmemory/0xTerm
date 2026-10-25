//! A pane's chrome: a small title when the tab is split, and a dim veil over
//! the panes that don't have focus, so you can see where your keys go.

use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::*;
use oxt_config::Chrome;
use oxt_session::alacritty_terminal::vte::ansi::Rgb;
use oxt_terminal_view::{Clear, Copy, Paste, TerminalView};
use oxt_toolbelt::{DraggedPath, Shell};

use crate::window::{ClosePane, SplitDown, SplitRight, Tab, Workspace};

pub type PaneId = u64;

pub struct Pane {
    pub view: Entity<TerminalView>,
    pub label: SharedString,
    pub _events: Subscription,
}

impl Pane {
    /// What to call a pane: the program in front (`vim`), else the folder (`~`, `src`).
    pub fn compute_label(view: &Entity<TerminalView>, cx: &App) -> SharedString {
        let session = view.read(cx).session();
        if session.is_busy()
            && let Some(process) = session.foreground()
        {
            return process.name.into();
        }
        let Some(cwd) = session.cwd() else { return "shell".into() };
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        if home.as_deref() == Some(cwd.as_path()) {
            return "~".into();
        }
        cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()).into()
    }

    /// How to quote a path for whatever is reading this pane's input: fish
    /// started from zsh gets fish quoting.
    pub fn shell(&self, cx: &App) -> Shell {
        self.view.read(cx).session().foreground().map(|p| Shell::from_process(&p.name)).unwrap_or_default()
    }
}

pub fn rgb_hsla(rgb: Rgb) -> Hsla {
    Rgba { r: rgb.r as f32 / 255., g: rgb.g as f32 / 255., b: rgb.b as f32 / 255., a: 1. }.into()
}

pub(crate) fn render_pane(pane: &Pane, id: PaneId, tab: &Tab, chrome: &Chrome, cx: &mut Context<Workspace>) -> AnyElement {
    let accent = chrome.accent;
    let split = tab.panes.len() > 1;
    let mut el = div().relative().flex().flex_col().size_full().min_w_0().min_h_0().overflow_hidden();
    if split {
        el = el.child(
            div()
                .flex_none()
                .h(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .text_color(rgba(chrome.dim))
                .bg(rgba(chrome.chrome))
                .border_b_1()
                .border_color(rgba(chrome.border))
                .overflow_hidden()
                .child(pane.label.clone()),
        );
    }
    el = el.child(
        div()
            .id(("pane", id))
            .flex_1()
            .min_h_0()
            .drag_over::<DraggedPath>(move |style, _, _, _| style.border_2().border_color(rgba(accent)))
            .on_drop(cx.listener(move |ws, dragged: &DraggedPath, window, cx| ws.drop_path(id, &dragged.0, window, cx)))
            .child(pane.view.clone())
            // Right-click focuses the pane first, so these act on it.
            .context_menu(|menu, _, _| {
                menu.menu("Copy", Box::new(Copy))
                    .menu("Paste", Box::new(Paste))
                    .separator()
                    .menu("Split Right", Box::new(SplitRight))
                    .menu("Split Down", Box::new(SplitDown))
                    .separator()
                    .menu("Clear Buffer", Box::new(Clear))
                    .menu("Close Pane", Box::new(ClosePane))
            }),
    );
    if split && tab.active != id {
        el = el.child(div().absolute().inset_0().bg(rgba(chrome.dim_pane)));
    }
    el.into_any_element()
}
