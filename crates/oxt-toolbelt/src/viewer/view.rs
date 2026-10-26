//! The right panel: open files as tabs. Code is a read-only highlighted
//! editor; Markdown has Preview/Source; images show as images. Files reload
//! when they change on disk, so `vim` in a pane updates the preview.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState, Position};
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use oxt_config::Chrome;

use crate::events::{OpenRequest, ToolbeltEvent};
use crate::viewer::load::{self, Content};
use crate::viewer::markdown::{self, Segment};
use crate::viewer::{find, mermaid};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Preview,
    Source,
}

struct Doc {
    path: PathBuf,
    content: Content,
    modified: Option<SystemTime>,
    mode: Mode,
    /// Code, or Markdown in Source mode.
    editor: Option<Entity<EditorState>>,
    segments: Vec<Segment>,
    preview_scroll: ScrollHandle,
}

impl Doc {
    fn is_markdown(&self) -> bool {
        matches!(self.content, Content::Markdown(_))
    }

    fn text(&self) -> Option<&str> {
        match &self.content {
            Content::Text { text, .. } | Content::Markdown(text) => Some(text),
            _ => None,
        }
    }

    fn uses_editor(&self) -> bool {
        matches!(self.content, Content::Text { .. }) || (self.is_markdown() && self.mode == Mode::Source)
    }
}

pub struct Viewer {
    docs: Vec<Doc>,
    active: usize,
    find: Entity<InputState>,
    find_count: SharedString,
    chrome: Chrome,
    dark: bool,
    /// Diagrams being drawn in the background.
    rendering: HashSet<(String, bool)>,
    /// Timed-out diagrams with a timer set for their second try.
    retrying: HashSet<(String, bool)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ToolbeltEvent> for Viewer {}

impl Viewer {
    pub fn new(chrome: Chrome, dark: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let subscriptions = vec![cx.subscribe_in(&find, window, |viewer, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => viewer.apply_find(cx),
            InputEvent::PressEnter { shift, .. } => viewer.step_find(!shift, window, cx),
            _ => {}
        })];

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(1500)).await;
                if this.update_in(cx, |viewer, window, cx| viewer.reload_changed(window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Viewer {
            docs: Vec::new(),
            active: 0,
            find,
            find_count: "".into(),
            chrome,
            dark,
            rendering: HashSet::new(),
            retrying: HashSet::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn set_theme(&mut self, chrome: Chrome, dark: bool, cx: &mut Context<Self>) {
        self.chrome = chrome;
        self.dark = dark;
        cx.notify();
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn focus_find(&self, window: &mut Window, cx: &mut App) {
        self.find.update(cx, |find, cx| find.focus(window, cx));
    }

    /// Opens a file (or switches to it), then jumps to the line and fills find.
    pub fn open(&mut self, request: OpenRequest, window: &mut Window, cx: &mut Context<Self>) {
        let index = match self.docs.iter().position(|d| d.path == request.path) {
            Some(i) => i,
            None => {
                let doc = self.load_doc(&request.path, window, cx);
                self.docs.push(doc);
                self.docs.len() - 1
            }
        };
        self.active = index;
        if let Some(line) = request.line {
            let doc = &mut self.docs[index];
            if doc.is_markdown() && doc.mode == Mode::Preview {
                doc.mode = Mode::Source;
                Self::ensure_editor(doc, window, cx);
            }
            if let Some(editor) = &doc.editor {
                let position = Position::new(line.saturating_sub(1) as u32, 0);
                editor.update(cx, |e, cx| e.set_cursor_position(position, window, cx));
            }
        }
        if let Some(query) = request.find {
            self.find.update(cx, |find, cx| find.set_value(query, window, cx));
        }
        self.apply_find(cx);
        cx.notify();
    }

    fn load_doc(&self, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> Doc {
        let content = load::load(path).unwrap_or_else(|e| Content::Text { text: format!("Couldn't read this file: {e}"), language: None });
        let mut doc = Doc {
            path: path.to_path_buf(),
            modified: modified(path),
            mode: Mode::Preview,
            editor: None,
            segments: Vec::new(),
            preview_scroll: ScrollHandle::new(),
            content,
        };
        if let Content::Markdown(text) = &doc.content {
            doc.segments = markdown::segments(text);
        }
        Self::ensure_editor(&mut doc, window, cx);
        doc
    }

    /// Creates the read-only editor a doc needs in its current mode.
    fn ensure_editor(doc: &mut Doc, window: &mut Window, cx: &mut Context<Self>) {
        if doc.editor.is_some() || !doc.uses_editor() {
            return;
        }
        let language = match &doc.content {
            Content::Text { language, .. } => language.unwrap_or("text"),
            _ => "markdown",
        };
        let text = doc.text().unwrap_or_default().to_owned();
        let editor = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx).language(language).line_number(true).soft_wrap(false);
            editor.set_value(text, window, cx);
            editor.set_readonly(true, cx);
            editor
        });
        doc.editor = Some(editor);
    }

    fn reload_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        for doc in &mut self.docs {
            let now = modified(&doc.path);
            if now.is_none() || now == doc.modified {
                continue;
            }
            doc.modified = now;
            if let Ok(content) = load::load(&doc.path) {
                if let Content::Markdown(text) = &content {
                    doc.segments = markdown::segments(text);
                }
                if let (Some(editor), Some(text)) = (
                    &doc.editor,
                    match &content {
                        Content::Text { text, .. } | Content::Markdown(text) => Some(text.clone()),
                        _ => None,
                    },
                ) {
                    editor.update(cx, |e, cx| e.set_value(text, window, cx));
                }
                doc.content = content;
                changed = true;
            }
        }
        if changed {
            self.apply_find(cx);
            cx.notify();
        }
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.docs.len() {
            self.docs.remove(index);
            if self.active >= self.docs.len() {
                self.active = self.docs.len().saturating_sub(1);
            }
            self.apply_find(cx);
            cx.notify();
        }
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(doc) = self.docs.get_mut(self.active) {
            doc.mode = mode;
            Self::ensure_editor(doc, window, cx);
        }
        self.apply_find(cx);
        cx.notify();
    }

    /// Highlights the find text in the editor and counts matches.
    fn apply_find(&mut self, cx: &mut Context<Self>) {
        let query = self.find.read(cx).value().to_string();
        let Some(doc) = self.docs.get(self.active) else {
            self.find_count = "".into();
            return;
        };
        if let (true, Some(editor)) = (doc.uses_editor(), &doc.editor) {
            let count = editor.update(cx, |e, cx| {
                if query.is_empty() {
                    e.close_search(cx);
                    0
                } else {
                    e.set_search_query(query.clone(), true, cx);
                    e.search_session().matcher.len()
                }
            });
            self.find_count = if query.is_empty() { "".into() } else { count.to_string().into() };
        } else {
            let count = doc.text().map(|t| find::find_all(t, &query).len()).unwrap_or(0);
            self.find_count = if query.is_empty() { "".into() } else { count.to_string().into() };
        }
        cx.notify();
    }

    fn step_find(&mut self, forward: bool, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.docs.get(self.active).and_then(|d| d.editor.clone()) {
            editor.update(cx, |e, cx| {
                if forward {
                    e.next_search_match(cx);
                } else {
                    e.previous_search_match(cx);
                }
            });
        }
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = self.chrome;
        let mut row = div()
            .flex()
            .flex_row()
            .gap(px(2.))
            .px(px(6.))
            .pt(px(4.))
            .bg(rgba(chrome.chrome))
            .border_b_1()
            .border_color(rgba(chrome.border))
            .overflow_hidden();
        for (i, doc) in self.docs.iter().enumerate() {
            let on = i == self.active;
            let name = doc.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            row = row.child(
                div()
                    .id(("vtab", i))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .pt(px(4.))
                    .pb(px(5.))
                    .rounded_t(px(6.))
                    .text_size(px(12.))
                    .whitespace_nowrap()
                    .text_color(rgba(if on { chrome.text } else { chrome.dim }))
                    .when(on, |d| d.bg(rgba(chrome.panel)))
                    .on_click(cx.listener(move |viewer, _, _, cx| {
                        viewer.active = i;
                        viewer.apply_find(cx);
                    }))
                    .child(name)
                    .child(
                        div()
                            .id(("vtab-close", i))
                            .size(px(16.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.6)
                            .hover(|s| s.bg(rgba(chrome.hover)).opacity(1.))
                            .on_click(cx.listener(move |viewer, _, _, cx| {
                                cx.stop_propagation();
                                viewer.close(i, cx);
                            }))
                            .child(svg().path("icons/x.svg").size(px(11.)).text_color(rgba(chrome.dim))),
                    ),
            );
        }
        row
    }

    fn render_bar(&self, doc: &Doc, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = self.chrome;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let shown = match home.as_deref().and_then(|h| doc.path.strip_prefix(h).ok()) {
            Some(rest) => format!("~/{}", rest.display()),
            None => doc.path.display().to_string(),
        };
        let seg = |label: &'static str, mode: Mode, cx: &mut Context<Self>| {
            let on = doc.mode == mode;
            div()
                .id(label)
                .px(px(8.))
                .py(px(1.))
                .rounded(px(5.))
                .text_size(px(12.))
                .text_color(rgba(if on { chrome.text } else { chrome.dim }))
                .when(on, |d| d.bg(rgba(chrome.panel)).shadow_sm())
                .on_click(cx.listener(move |viewer, _, window, cx| viewer.set_mode(mode, window, cx)))
                .child(label)
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(rgba(chrome.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .text_color(rgba(chrome.faint))
                    .child(shown),
            )
            .when(doc.is_markdown(), |d| {
                d.child(
                    div().flex().gap(px(2.)).p(px(2.)).rounded(px(7.)).bg(rgba(chrome.chrome_2)).child(seg("Preview", Mode::Preview, cx)).child(seg(
                        "Source",
                        Mode::Source,
                        cx,
                    )),
                )
            })
            .child(div().w(px(170.)).child(Input::new(&self.find).cleanable(true)))
            .child(div().min_w(px(32.)).text_size(px(11.)).text_color(rgba(chrome.faint)).child(self.find_count.clone()))
    }

    fn render_preview(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let dark = self.dark;
        let chrome = self.chrome;
        let doc = &self.docs[index];
        let base = doc.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let viewer = cx.entity();
        let mut column = div().flex().flex_col().gap(px(12.)).px(px(28.)).py(px(20.)).max_w(px(860.));
        let mut to_render = Vec::new();
        let mut to_retry = Vec::new();
        for (i, segment) in doc.segments.iter().enumerate() {
            match segment {
                Segment::Markdown(text) => {
                    let (viewer, base) = (viewer.clone(), base.clone());
                    column = column.child(
                        TextView::markdown(SharedString::from(format!("md-{index}-{i}")), text.clone())
                            .selectable(true)
                            .on_link_click(move |link, _, window, cx| open_link(&viewer, &base, link, window, cx)),
                    );
                }
                Segment::Mermaid(source) => match mermaid::cached(source, dark) {
                    Some(Ok(diagram)) => {
                        let image = Arc::new(Image::from_bytes(ImageFormat::Png, (*diagram.png).clone()));
                        column = column.child(
                            div()
                                .flex()
                                .justify_center()
                                .child(img(image).w(px(diagram.width)).h(px(diagram.height)).max_w_full().object_fit(ObjectFit::Contain)),
                        );
                    }
                    Some(Err(err)) => {
                        if let Some(at) = mermaid::retry_at(source, dark) {
                            to_retry.push((source.clone(), at));
                        }
                        column = column.child(
                            div()
                                .p(px(10.))
                                .rounded(px(6.))
                                .bg(rgba(chrome.panel))
                                .text_size(px(12.))
                                .font_family("Menlo")
                                .text_color(rgba(chrome.dim))
                                .child(match err {
                                    // The source may be what made it too large; don't lay it out.
                                    mermaid::RenderError::TooLarge(_) | mermaid::RenderError::TookTooLong(_) => format!("Mermaid: {err}"),
                                    mermaid::RenderError::Invalid(_) => format!("Mermaid: {err}\n\n{source}"),
                                }),
                        );
                    }
                    None => {
                        to_render.push(source.clone());
                        column = column.child(div().text_size(px(12.)).text_color(rgba(chrome.faint)).child("Drawing diagram…"));
                    }
                },
            }
        }
        for (source, at) in to_retry {
            if self.retrying.insert((source.clone(), dark)) {
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(at.saturating_duration_since(Instant::now())).await;
                    let _ = this.update(cx, |viewer, cx| {
                        viewer.retrying.remove(&(source, dark));
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        for source in to_render {
            if self.rendering.insert((source.clone(), dark)) {
                cx.spawn_in(window, async move |this, cx| {
                    let src = source.clone();
                    cx.background_executor().spawn(async move { mermaid::render(&src, dark) }).await.ok();
                    let _ = this.update(cx, |viewer, cx| {
                        viewer.rendering.remove(&(source, dark));
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        let doc = &self.docs[index];
        div().id(("preview", index)).size_full().overflow_y_scroll().track_scroll(&doc.preview_scroll).child(column).into_any_element()
    }
}

/// Where a Markdown link goes.
#[derive(Debug, PartialEq)]
enum LinkTarget {
    /// A URL: the window decides where it may go, and asks first.
    Url(String),
    /// A tab in this viewer.
    File(PathBuf),
}

/// Relative links open in the viewer; URLs go to the window.
fn link_target(base: &Path, link: &str) -> Option<LinkTarget> {
    if url::Url::parse(link).is_ok() {
        return Some(LinkTarget::Url(link.to_owned()));
    }
    let target = link.split('#').next().unwrap_or_default();
    if target.is_empty() {
        return None;
    }
    Some(LinkTarget::File(base.join(target)))
}

fn open_link(viewer: &Entity<Viewer>, base: &Path, link: &SharedString, window: &mut Window, cx: &mut App) {
    match link_target(base, link) {
        Some(LinkTarget::Url(url)) => viewer.update(cx, |_, cx| cx.emit(ToolbeltEvent::OpenUrl(url))),
        Some(LinkTarget::File(path)) => viewer.update(cx, |viewer, cx| viewer.open(OpenRequest::path(path), window, cx)),
        None => {}
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = self.chrome;
        if self.docs.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(rgba(chrome.faint))
                .child("Click a file to open it here")
                .into_any_element();
        }
        let index = self.active.min(self.docs.len() - 1);
        let body: AnyElement = {
            let doc = &self.docs[index];
            match (&doc.content, doc.mode) {
                (Content::Markdown(_), Mode::Preview) => self.render_preview(index, window, cx),
                (Content::Text { .. }, _) | (Content::Markdown(_), Mode::Source) => match &doc.editor {
                    Some(editor) => Editor::new(editor).bordered(false).h_full().into_any_element(),
                    None => div().into_any_element(),
                },
                (Content::Image, _) => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(16.))
                    .child(img(doc.path.clone()).max_w_full().max_h_full().object_fit(ObjectFit::Contain))
                    .into_any_element(),
                (Content::Binary, _) => notice("Binary file — not shown", chrome),
                (Content::TooLarge(size), _) => notice(&format!("Too large to show ({:.1} MB, limit 2 MB)", *size as f64 / 1_048_576.), chrome),
            }
        };
        let tabs = self.render_tabs(cx).into_any_element();
        let bar = self.render_bar(&self.docs[index], cx).into_any_element();
        div().size_full().flex().flex_col().child(tabs).child(bar).child(div().flex_1().min_h_0().child(body)).into_any_element()
    }
}

fn notice(text: &str, chrome: Chrome) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(rgba(chrome.faint))
        .child(text.to_owned())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob brings its own `test` attribute.
    use super::{LinkTarget, link_target};
    use std::path::Path;

    #[test]
    fn urls_go_to_the_window_and_relative_links_open_here() {
        let base = Path::new("/repo/docs");
        for url in ["https://0xterm.org/docs/", "mailto:a@b.c", "FILE:///repo/run.command", "smb://evil/share"] {
            assert_eq!(link_target(base, url), Some(LinkTarget::Url(url.into())));
        }
        assert_eq!(link_target(base, "guide.md#setup"), Some(LinkTarget::File("/repo/docs/guide.md".into())));
        assert_eq!(link_target(base, "#top"), None);
    }
}
