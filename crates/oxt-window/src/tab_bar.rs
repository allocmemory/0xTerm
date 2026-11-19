//! The title row: room for the traffic lights, the tabs, a + button, and the
//! two panel toggles, so the title bar takes no extra space.

use gpui_kit::component::input::{Escape, Input};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::window::Workspace;

pub(crate) fn render_title_row(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let chrome = ws.chrome;
    let workspace = cx.entity();
    let mut tabs = div().flex().flex_row().gap(px(2.)).min_w_0().overflow_hidden();
    for (i, tab) in ws.tabs.iter().enumerate() {
        let on = i == ws.active;
        let label = tab.label();
        let tab_id = tab.id;
        let renaming = ws.renaming.as_ref().filter(|r| r.tab == tab_id).map(|r| r.input.clone());
        let name: AnyElement = match &renaming {
            Some(input) => div()
                .flex_1()
                .min_w_0()
                .on_action(cx.listener(|ws, _: &Escape, window, cx| ws.finish_rename(false, window, cx)))
                .child(Input::new(input).appearance(false).h(px(22.)).px_0())
                .into_any_element(),
            None => div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(label).into_any_element(),
        };
        let menu_ws = workspace.clone();
        let editing = renaming.is_some();
        tabs = tabs.child(
            div()
                .id(("tab", i))
                .group("tab")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .h(px(26.))
                .min_w(px(90.))
                .max_w(px(220.))
                .pl(px(10.))
                .pr(px(6.))
                .rounded(px(6.))
                .text_color(rgba(if on { chrome.text } else { chrome.dim }))
                .when(on, |d| d.bg(rgba(chrome.chrome_2)))
                .when(!on, |d| d.hover(|s| s.bg(rgba(chrome.hover))))
                // Room to type while renaming.
                .when(editing, |d| d.min_w(px(180.)))
                .on_click(cx.listener(move |ws, event: &ClickEvent, window, cx| {
                    // Clicks in the name field are for the field, and a
                    // double-click there selects a word, never zooms the window.
                    if ws.renaming.as_ref().is_some_and(|r| r.tab == tab_id) {
                        cx.stop_propagation();
                        return;
                    }
                    if event.modifiers().platform {
                        return;
                    }
                    ws.activate_tab(i, window, cx);
                    if event.click_count() >= 2 {
                        // Rename instead of zooming the window, which the title row does on a double-click.
                        cx.stop_propagation();
                        ws.start_rename(i, window, cx);
                    }
                }))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |ws, _, window, cx| ws.close_tab(i, window, cx)))
                // The menu outlives this frame, so it finds its tab by id when
                // clicked: tabs may have closed and shifted while it was open.
                .context_menu(move |menu, _, _| {
                    let (rename_ws, close_ws) = (menu_ws.clone(), menu_ws.clone());
                    menu.item(PopupMenuItem::new("Rename Tab…").on_click(move |_, window, cx| {
                        rename_ws.update(cx, |ws, cx| {
                            if let Some(i) = ws.tab_index(tab_id) {
                                ws.activate_tab(i, window, cx);
                                ws.start_rename(i, window, cx);
                            }
                        })
                    }))
                    .item(PopupMenuItem::new("Close Tab").on_click(move |_, window, cx| {
                        close_ws.update(cx, |ws, cx| {
                            if let Some(i) = ws.tab_index(tab_id) {
                                ws.close_tab(i, window, cx);
                            }
                        })
                    }))
                })
                .child(name)
                .when(!editing, |d| {
                    d.child(
                        div()
                            .id(("tab-close", i))
                            .flex_none()
                            .size(px(18.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(if on { 0.7 } else { 0. })
                            .group_hover("tab", |s| s.opacity(0.7))
                            .hover(|s| s.bg(rgba(chrome.hover)).opacity(1.))
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                cx.stop_propagation();
                                ws.close_tab(i, window, cx);
                            }))
                            .child(icon("icons/x.svg", 12., chrome.dim)),
                    )
                }),
        );
    }

    let button = |id: &'static str, pressed: bool| {
        div()
            .id(id)
            .flex_none()
            .h(px(26.))
            .min_w(px(26.))
            .px(px(5.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .hover(|s| s.bg(rgba(chrome.hover)))
            .text_color(rgba(if pressed { chrome.text } else { chrome.dim }))
    };

    div()
        .id("title-row")
        .flex_none()
        .h(px(36.))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        // The macOS traffic lights live in this space.
        .pl(px(80.))
        .pr(px(8.))
        .bg(rgba(chrome.chrome))
        .border_b_1()
        .border_color(rgba(chrome.border))
        .window_control_area(WindowControlArea::Drag)
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.titlebar_double_click();
            }
        })
        .child(tabs)
        .child(button("tab-add", false).on_click(cx.listener(|ws, _, window, cx| ws.dispatch_new_tab(window, cx))).child(icon(
            "icons/plus.svg",
            16.,
            chrome.dim,
        )))
        .child(div().flex_1())
        .child(button("toggle-sidebar", ws.sidebar_open).on_click(cx.listener(|ws, _, window, cx| ws.toggle_sidebar(window, cx))).child(icon(
            "icons/panel-left.svg",
            16.,
            if ws.sidebar_open { chrome.text } else { chrome.dim },
        )))
        .child(button("toggle-viewer", ws.viewer_open).on_click(cx.listener(|ws, _, window, cx| ws.toggle_viewer(window, cx))).child(icon(
            "icons/panel-right.svg",
            16.,
            if ws.viewer_open { chrome.text } else { chrome.dim },
        )))
}

fn icon(path: &'static str, size: f32, color: u32) -> impl IntoElement {
    svg().path(path).size(px(size)).flex_none().text_color(rgba(color))
}
