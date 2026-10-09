//! Folder search: type, wait a beat, see every hit grouped by file. Clicking a
//! hit opens the file at that line with the query in the viewer's find box.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use oxt_config::Chrome;

use crate::events::{OpenRequest, ToolbeltEvent};
use crate::search::{self, Results};

enum Row {
    File { path: PathBuf, count: usize },
    Hit { path: PathBuf, line: u64, text: SharedString },
}

pub struct SearchPanel {
    root: PathBuf,
    input: Entity<InputState>,
    rows: Vec<Row>,
    status: SharedString,
    query: String,
    cancel: Arc<AtomicBool>,
    pub(crate) chrome: Chrome,
    _input_events: Subscription,
}

impl EventEmitter<ToolbeltEvent> for SearchPanel {}

impl SearchPanel {
    pub fn new(root: PathBuf, chrome: Chrome, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search in folder"));
        let events = cx.subscribe_in(&input, window, |panel, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                panel.schedule(cx);
            }
        });
        SearchPanel {
            root,
            input,
            rows: Vec::new(),
            status: "".into(),
            query: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            chrome,
            _input_events: events,
        }
    }

    pub fn input(&self) -> Entity<InputState> {
        self.input.clone()
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if root != self.root {
            self.root = root;
            self.schedule(cx);
        }
    }

    /// Debounced: a new keystroke cancels the search in flight.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let query = self.input.read(cx).value().to_string();
        let root = self.root.clone();
        if query.trim().is_empty() {
            self.rows.clear();
            self.status = "".into();
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let (q, flag) = (query.clone(), cancel.clone());
            let results = cx.background_executor().spawn(async move { search::search(&root, &q, &flag) }).await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let _ = this.update(cx, |panel, cx| panel.show(query, results, cx));
        })
        .detach();
    }

    fn show(&mut self, query: String, results: Results, cx: &mut Context<Self>) {
        self.rows.clear();
        for file in results.files {
            self.rows.push(Row::File { path: file.path.clone(), count: file.hits.len() });
            for hit in file.hits {
                self.rows.push(Row::Hit { path: file.path.clone(), line: hit.line, text: hit.text.into() });
            }
        }
        let files = self.rows.iter().filter(|r| matches!(r, Row::File { .. })).count();
        let more = if results.truncated { "+" } else { "" };
        self.status = match results.total {
            0 => "No results".into(),
            n => format!("{n}{more} results in {files} files").into(),
        };
        self.query = query;
        cx.notify();
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let chrome = self.chrome;
        match &self.rows[index] {
            Row::File { path, count } => {
                let rel = path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().into_owned();
                let open = path.clone();
                div()
                    .id(("search-row", index))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .text_size(px(12.))
                    .text_color(rgba(chrome.text))
                    .hover(|s| s.bg(rgba(chrome.hover)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(ToolbeltEvent::Open(OpenRequest::path(open.clone())))))
                    .child(svg().path("icons/file.svg").size(px(13.)).flex_none().text_color(rgba(chrome.dim)))
                    .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(rel))
                    .child(div().flex_none().text_color(rgba(chrome.faint)).child(count.to_string()))
                    .into_any_element()
            }
            Row::Hit { path, line, text } => {
                let request = OpenRequest { path: path.clone(), line: Some(*line), find: Some(self.query.clone()) };
                div()
                    .id(("search-row", index))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(26.))
                    .pr(px(8.))
                    .text_size(px(12.))
                    .hover(|s| s.bg(rgba(chrome.hover)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(ToolbeltEvent::Open(request.clone()))))
                    .child(div().flex_none().w(px(36.)).text_right().text_color(rgba(chrome.faint)).child(line.to_string()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(rgba(chrome.dim))
                            .child(text.clone()),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.rows.len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().px(px(8.)).pt(px(2.)).child(Input::new(&self.input).cleanable(true)))
            .child(
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(11.))
                    .text_color(rgba(self.chrome.faint))
                    .when(!self.status.is_empty(), |d| d.child(self.status.clone())),
            )
            .child(
                div().flex_1().min_h_0().child(
                    uniform_list(
                        "search-results",
                        count,
                        cx.processor(|panel, range: std::ops::Range<usize>, _, cx| range.map(|i| panel.render_row(i, cx)).collect::<Vec<_>>()),
                    )
                    .size_full(),
                ),
            )
    }
}
