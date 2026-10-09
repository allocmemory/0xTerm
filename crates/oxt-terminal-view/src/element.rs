//! The terminal's custom GPUI element. Once per frame it locks the engine,
//! turns the visible grid into quads and shaped text, unlocks, then paints.
//! GPUI's scene (quads + glyph atlas) covers everything a terminal needs.

use gpui_kit::*;
use oxt_session::Engine;
use oxt_session::alacritty_terminal::term::cell::Flags;
use oxt_session::alacritty_terminal::vte::ansi::{CursorShape, Rgb};

use crate::style::{CellSize, TerminalStyle, dim, hsla};
use crate::view::TerminalView;

pub struct TerminalElement {
    view: Entity<TerminalView>,
    focus: FocusHandle,
}

impl TerminalElement {
    pub fn new(view: Entity<TerminalView>, focus: FocusHandle) -> Self {
        TerminalElement { view, focus }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// Everything painted in one frame, built while the engine is locked.
pub struct Frame {
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Vec<(Bounds<Pixels>, Hsla)>,
    cursor_text: Option<(Point<Pixels>, ShapedLine)>,
    line_height: Pixels,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Frame;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Frame {
        let focused = self.focus.is_focused(window);
        self.view.update(cx, |view, _| view.frame(bounds, focused, window))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        frame: &mut Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(&self.focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (rect, color) in &frame.rects {
                window.paint_quad(fill(*rect, *color));
            }
            for (origin, line) in &frame.lines {
                let _ = line.paint(*origin, frame.line_height, TextAlign::Left, None, window, cx);
            }
            for (rect, color) in &frame.cursor {
                window.paint_quad(fill(*rect, *color));
            }
            if let Some((origin, line)) = &frame.cursor_text {
                let _ = line.paint(*origin, frame.line_height, TextAlign::Left, None, window, cx);
            }
        });
    }
}

/// A run of cells shaped as one line of text, starting at `col`.
struct Batch {
    line: usize,
    col: usize,
    text: String,
    runs: Vec<TextRun>,
    /// One wide or non-ASCII cell: shaped on its own so fallback fonts can't push the grid.
    single: bool,
}

/// Builds the frame from the engine. Called with the engine locked; keep it lean.
#[allow(clippy::too_many_arguments)]
pub fn build_frame(
    engine: &Engine,
    style: &TerminalStyle,
    cell: CellSize,
    origin: Point<Pixels>,
    focused: bool,
    marked: Option<&str>,
    window: &mut Window,
) -> (Frame, Bounds<Pixels>) {
    let content = engine.term.renderable_content();
    let colors = content.colors;
    let offset = content.display_offset as i32;
    let default_bg = style.palette.background;
    let base_font = style.font();
    let text_system = window.text_system().clone();

    let cell_bounds = |line: usize, col: usize, cols: usize| {
        Bounds::new(point(origin.x + cell.width * col as f32, origin.y + cell.height * line as f32), size(cell.width * cols as f32, cell.height))
    };

    let mut rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let mut lines = Vec::new();
    let mut batch: Option<Batch> = None;
    let mut cursor_glyph: Option<(char, Rgb, Font)> = None;
    let cursor = content.cursor;
    let cursor_line = cursor.point.line.0 + offset;

    let shape = |batch: Batch, lines: &mut Vec<(Point<Pixels>, ShapedLine)>| {
        let force = (!batch.single).then_some(cell.width);
        let shaped = text_system.shape_line(batch.text.into(), style.font_size, &batch.runs, force);
        lines.push((cell_bounds(batch.line, batch.col, 1).origin, shaped));
    };

    for indexed in content.display_iter {
        let cell_ref = indexed.cell;
        let flags = cell_ref.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let line = (indexed.point.line.0 + offset) as usize;
        let col = indexed.point.column.0;
        let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

        let mut fg = style.color(cell_ref.fg, colors);
        let mut bg = style.color(cell_ref.bg, colors);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = dim(fg);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let selected = content.selection.is_some_and(|s| s.contains(indexed.point));
        if selected {
            bg = style.selection;
        }

        if bg != default_bg {
            let rect = cell_bounds(line, col, width);
            let color = hsla(bg);
            match rects.last_mut() {
                Some((last, last_color)) if *last_color == color && last.origin.y == rect.origin.y && last.right() == rect.left() => {
                    last.size.width += rect.size.width;
                }
                _ => rects.push((rect, color)),
            }
        }

        let mut font = base_font.clone();
        if flags.contains(Flags::BOLD) {
            font.weight = FontWeight::BOLD;
        }
        if flags.contains(Flags::ITALIC) {
            font.style = FontStyle::Italic;
        }

        let c = cell_ref.c;
        if line as i32 == cursor_line && col == cursor.point.column.0 {
            cursor_glyph = Some((c, bg, font.clone()));
        }
        if c == ' ' && !flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT) {
            // Blank cells only need their background; end the batch so text stays aligned.
            if let Some(b) = batch.take() {
                shape(b, &mut lines);
            }
            continue;
        }

        let mut text = String::from(c);
        let combining = cell_ref.zerowidth();
        if let Some(extra) = combining {
            text.extend(extra);
        }
        let single = width == 2 || !c.is_ascii() || combining.is_some();
        let fg_color = hsla(fg);
        let run = TextRun {
            len: text.len(),
            font,
            color: fg_color,
            background_color: None,
            underline: flags.intersects(Flags::ALL_UNDERLINES).then(|| UnderlineStyle {
                thickness: px(1.),
                color: Some(fg_color),
                wavy: flags.contains(Flags::UNDERCURL),
            }),
            strikethrough: flags.contains(Flags::STRIKEOUT).then(|| StrikethroughStyle { thickness: px(1.), color: Some(fg_color) }),
        };

        let continues = batch.as_ref().is_some_and(|b| !b.single && !single && b.line == line && b.col + b.text_cells() == col);
        if !continues && let Some(b) = batch.take() {
            shape(b, &mut lines);
        }
        match &mut batch {
            Some(b) => {
                b.text.push_str(&text);
                match b.runs.last_mut() {
                    Some(last) if same_style(last, &run) => last.len += run.len,
                    _ => b.runs.push(run),
                }
            }
            None => batch = Some(Batch { line, col, text, runs: vec![run], single }),
        }
    }
    if let Some(b) = batch.take() {
        shape(b, &mut lines);
    }

    // Cursor. Unfocused panes show a hollow block.
    let mut cursor_rects = Vec::new();
    let mut cursor_text = None;
    let mut ime_bounds = cell_bounds(0, 0, 1);
    if cursor_line >= 0 {
        let rect = cell_bounds(cursor_line as usize, cursor.point.column.0, 1);
        ime_bounds = rect;
        let color = hsla(style.palette.cursor);
        let thin = px(2.);
        let shape = if focused {
            cursor.shape
        } else if cursor.shape == CursorShape::Hidden {
            CursorShape::Hidden
        } else {
            CursorShape::HollowBlock
        };
        match shape {
            CursorShape::Block => {
                cursor_rects.push((rect, color));
                if let Some((c, bg, font)) = cursor_glyph.filter(|(c, ..)| *c != ' ') {
                    let text: SharedString = c.to_string().into();
                    let run = TextRun { len: text.len(), font, color: hsla(bg), background_color: None, underline: None, strikethrough: None };
                    cursor_text = Some((rect.origin, text_system.shape_line(text, style.font_size, &[run], None)));
                }
            }
            CursorShape::Beam => cursor_rects.push((Bounds::new(rect.origin, size(thin, rect.size.height)), color)),
            CursorShape::Underline => cursor_rects.push((Bounds::new(point(rect.left(), rect.bottom() - thin), size(rect.size.width, thin)), color)),
            CursorShape::HollowBlock => {
                let one = px(1.);
                cursor_rects.push((Bounds::new(rect.origin, size(rect.size.width, one)), color));
                cursor_rects.push((Bounds::new(point(rect.left(), rect.bottom() - one), size(rect.size.width, one)), color));
                cursor_rects.push((Bounds::new(rect.origin, size(one, rect.size.height)), color));
                cursor_rects.push((Bounds::new(point(rect.right() - one, rect.top()), size(one, rect.size.height)), color));
            }
            CursorShape::Hidden => {}
        }

        // IME composition shows at the cursor, underlined, until it's committed.
        if let Some(marked) = marked.filter(|m| !m.is_empty()) {
            let fg = hsla(style.palette.foreground);
            let run = TextRun {
                len: marked.len(),
                font: base_font.clone(),
                color: fg,
                background_color: None,
                underline: Some(UnderlineStyle { thickness: px(1.), color: Some(fg), wavy: false }),
                strikethrough: None,
            };
            let shaped = text_system.shape_line(marked.to_owned().into(), style.font_size, &[run], None);
            let width = shaped.width.max(cell.width);
            cursor_rects = vec![(Bounds::new(rect.origin, size(width, rect.size.height)), hsla(default_bg))];
            cursor_text = Some((rect.origin, shaped));
        }
    }

    (Frame { rects, lines, cursor: cursor_rects, cursor_text, line_height: cell.height }, ime_bounds)
}

impl Batch {
    /// Cells covered so far. Batches only hold ASCII, so bytes = cells.
    fn text_cells(&self) -> usize {
        self.text.len()
    }
}

fn same_style(a: &TextRun, b: &TextRun) -> bool {
    a.font == b.font && a.color == b.color && a.underline == b.underline && a.strikethrough == b.strikethrough
}
