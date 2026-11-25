use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Point as GridPoint};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode, cell::Flags, viewport_to_point};
use alacritty_terminal::vte::ansi::{Color, CursorShape};
use gpui_kit::*;
use lanes_core::dropped_paths::dropped_paths_text;
use uuid::Uuid;

use crate::keys::{Mods, bracketed_paste, encode_key};
use crate::palette::{self, Palette, to_hsla};
use crate::pty::{GridSize, Listener, PtySession};

pub const KEY_CONTEXT: &str = "Terminal";
const FONT_FAMILY: &str = "Cascadia Mono";
const FONT_SIZE: f32 = 14.0;
/// Wheel lines that add up to one PgUp/PgDn sent to a full-screen app (one wheel notch on Windows).
const LINES_PER_PAGE_KEY: f32 = 3.0;

/// Cell geometry from the last layout, shared between painting and mouse handling.
#[derive(Clone, Copy, Default)]
struct Geometry {
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
}

pub struct TerminalView {
    pub session_id: Uuid,
    pub session: Arc<PtySession>,
    focus: FocusHandle,
    geometry: Rc<Cell<Geometry>>,
    selecting: bool,
    scroll_remainder: f32,
}

impl TerminalView {
    pub fn new(session_id: Uuid, session: Arc<PtySession>, cx: &mut Context<Self>) -> Self {
        let view = Self { session_id, session, focus: cx.focus_handle(), geometry: Rc::default(), selecting: false, scroll_remainder: 0.0 };
        view.start_repaint_loop(cx);
        view
    }

    pub fn send_text(&self, text: &str) {
        self.type_text(text);
        self.session.write(b"\r");
    }

    /// Puts text in the input box without submitting it.
    pub fn type_text(&self, text: &str) {
        let bracketed = self.session.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.session.write(&bracketed_paste(text, bracketed));
    }

    fn start_repaint_loop(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let alive = this.update(cx, |view, cx| {
                    if view.session.take_dirty() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        match (modifiers.control, modifiers.shift, keystroke.key.as_str()) {
            (true, true, "c") => self.copy_selection(cx),
            (true, _, "v") => self.paste(cx),
            (false, true, "pageup") => self.scroll(Scroll::PageUp),
            (false, true, "pagedown") => self.scroll(Scroll::PageDown),
            _ => self.send_keystroke(keystroke),
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn send_keystroke(&mut self, keystroke: &Keystroke) {
        let mods = Mods { ctrl: keystroke.modifiers.control, alt: keystroke.modifiers.alt, shift: keystroke.modifiers.shift };
        let app_cursor = self.session.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = encode_key(&keystroke.key, keystroke.key_char.as_deref(), mods, app_cursor) {
            {
                let mut term = self.session.term.lock();
                term.selection = None;
                term.scroll_display(Scroll::Bottom);
            }
            self.session.write(&bytes);
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        match cx.read_from_clipboard().and_then(|item| item.text()) {
            Some(text) => {
                let bracketed = self.session.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
                self.session.write(&bracketed_paste(&text, bracketed));
            }
            // No text on the clipboard: pass Ctrl+V through so claude can paste an image itself.
            None => self.session.write(&[0x16]),
        }
    }

    fn on_drop_paths(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.type_text(&dropped_paths_text(paths.paths()));
        cx.notify();
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let text = self.session.term.lock().selection_to_string();
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn scroll(&mut self, scroll: Scroll) {
        self.session.term.lock().scroll_display(scroll);
        self.session.mark_dirty();
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let alternate = self.session.term.lock().mode().contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL);
        let line_height = self.geometry.get().line_height.max(px(1.));
        let lines_per_step = if alternate { LINES_PER_PAGE_KEY } else { 1.0 };
        self.scroll_remainder += event.delta.pixel_delta(line_height).y / line_height / lines_per_step;
        let steps = self.scroll_remainder.trunc() as i32;
        if steps == 0 {
            return;
        }
        self.scroll_remainder -= steps as f32;
        if alternate {
            // Full-screen apps like Claude Code treat arrows as prompt history, so scroll them with page keys.
            let key: &[u8] = if steps > 0 { b"\x1b[5~" } else { b"\x1b[6~" };
            self.session.write(&key.repeat(steps.unsigned_abs() as usize));
        } else {
            self.scroll(Scroll::Delta(steps));
        }
        cx.notify();
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let (point, side) = self.grid_point(event.position);
        self.session.term.lock().selection = Some(Selection::new(SelectionType::Simple, point, side));
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (point, side) = self.grid_point(event.position);
        if let Some(selection) = self.session.term.lock().selection.as_mut() {
            selection.update(point, side);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        let text = self.session.term.lock().selection_to_string();
        match text.filter(|t| !t.trim().is_empty()) {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            None => self.session.term.lock().selection = None,
        }
        cx.notify();
    }

    fn grid_point(&self, position: Point<Pixels>) -> (GridPoint, Direction) {
        let geometry = self.geometry.get();
        let size = self.session.size();
        let x = ((position.x - geometry.origin.x) / geometry.cell_width.max(px(1.))).max(0.0);
        let y = ((position.y - geometry.origin.y) / geometry.line_height.max(px(1.))).max(0.0);
        let column = (x as usize).min(size.columns.saturating_sub(1));
        let row = (y as usize).min(size.rows.saturating_sub(1));
        let side = if x.fract() > 0.5 { Direction::Right } else { Direction::Left };
        let offset = self.session.term.lock().grid().display_offset();
        (viewport_to_point(offset, GridPoint::new(row, Column(column))), side)
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.clone();
        let geometry = self.geometry.clone();
        let focused = self.focus.is_focused(window);
        let palette = palette::active(cx);
        div()
            .id(("terminal", self.session_id.as_u128() as u64))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(to_hsla(palette.background))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_drop(cx.listener(Self::on_drop_paths))
            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(0.8))
            .child(
                canvas(
                    move |bounds, window, _cx| prepare_frame(&session, &geometry, bounds, palette, window),
                    move |_bounds, frame, window, cx| paint_frame(frame, focused, window, cx),
                )
                .size_full(),
            )
    }
}

#[derive(Clone, Copy, PartialEq)]
struct CellStyle {
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

struct Span {
    column: usize,
    cells: usize,
    text: String,
    style: CellStyle,
    wide: bool,
}

struct Frame {
    geometry: Geometry,
    rows: Vec<Vec<Span>>,
    cursor: Option<(usize, usize, CursorShape)>,
    cursor_color: Hsla,
}

fn prepare_frame(session: &PtySession, geometry: &Cell<Geometry>, bounds: Bounds<Pixels>, palette: &Palette, window: &mut Window) -> Frame {
    let metrics = measure_cells(bounds.origin, window);
    geometry.set(metrics);
    let columns = (bounds.size.width / metrics.cell_width).floor().max(2.0) as usize;
    let rows = (bounds.size.height / metrics.line_height).floor().max(2.0) as usize;
    session.resize(GridSize { columns, rows });
    let term = session.term.lock();
    snapshot(&term, metrics, palette)
}

fn measure_cells(origin: Point<Pixels>, window: &mut Window) -> Geometry {
    let font_size = px(FONT_SIZE);
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font(FONT_FAMILY));
    let cell_width = text_system.em_advance(font_id, font_size).unwrap_or(px(FONT_SIZE * 0.6));
    Geometry { origin, cell_width, line_height: (font_size * 1.3).round() }
}

fn snapshot(term: &Term<Listener>, geometry: Geometry, palette: &Palette) -> Frame {
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let screen_rows = term.screen_lines();
    let mut rows: Vec<Vec<Span>> = (0..screen_rows).map(|_| Vec::new()).collect();
    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        let cell = indexed.cell;
        if row < 0 || row as usize >= screen_rows || cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let selected = content.selection.is_some_and(|s| s.contains(indexed.point));
        let style = cell_style(cell.fg, cell.bg, cell.flags, content.colors, selected, palette);
        let mut text = String::from(if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c });
        if let Some(extra) = cell.zerowidth() {
            text.extend(extra);
        }
        push_cell(&mut rows[row as usize], indexed.point.column.0, text, style, cell.flags.contains(Flags::WIDE_CHAR));
    }
    let cursor = content.cursor;
    let cursor_row = cursor.point.line.0 + offset;
    let visible = cursor.shape != CursorShape::Hidden && cursor_row >= 0 && (cursor_row as usize) < screen_rows;
    let cursor_color = to_hsla(palette.cursor);
    Frame { geometry, rows, cursor: visible.then_some((cursor_row as usize, cursor.point.column.0, cursor.shape)), cursor_color }
}

fn push_cell(row: &mut Vec<Span>, column: usize, text: String, style: CellStyle, wide: bool) {
    if let Some(last) = row.last_mut()
        && !wide
        && !last.wide
        && last.style == style
        && last.column + last.cells == column
    {
        last.text.push_str(&text);
        last.cells += 1;
        return;
    }
    row.push(Span { column, cells: if wide { 2 } else { 1 }, text, style, wide });
}

fn paint_frame(frame: Frame, focused: bool, window: &mut Window, cx: &mut App) {
    let Geometry { origin, cell_width, line_height } = frame.geometry;
    let cell_origin = |row: usize, column: usize| point(origin.x + cell_width * column as f32, origin.y + line_height * row as f32);
    for (row, spans) in frame.rows.iter().enumerate() {
        for span in spans {
            if let Some(bg) = span.style.bg {
                window.paint_quad(fill(Bounds::new(cell_origin(row, span.column), size(cell_width * span.cells as f32, line_height)), bg));
            }
        }
    }
    if let Some((row, column, shape)) = frame.cursor {
        paint_cursor(cell_origin(row, column), cell_width, line_height, shape, frame.cursor_color, focused, window);
    }
    for (row, spans) in frame.rows.iter().enumerate() {
        for span in spans.iter().filter(|s| !s.text.trim().is_empty()) {
            paint_text(span, cell_origin(row, span.column), line_height, window, cx);
        }
    }
}

fn paint_text(span: &Span, origin: Point<Pixels>, line_height: Pixels, window: &mut Window, cx: &mut App) {
    let mut run_font = font(FONT_FAMILY);
    if span.style.bold {
        run_font.weight = FontWeight::BOLD;
    }
    if span.style.italic {
        run_font.style = FontStyle::Italic;
    }
    let run = TextRun {
        len: span.text.len(),
        font: run_font,
        color: span.style.fg,
        background_color: None,
        underline: span.style.underline.then(|| UnderlineStyle { color: Some(span.style.fg), thickness: px(1.), wavy: false }),
        strikethrough: span.style.strike.then(|| StrikethroughStyle { color: Some(span.style.fg), thickness: px(1.) }),
    };
    let line = window.text_system().shape_line(span.text.clone().into(), px(FONT_SIZE), &[run], None);
    let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
}

fn paint_cursor(origin: Point<Pixels>, cell_width: Pixels, line_height: Pixels, shape: CursorShape, color: Hsla, focused: bool, window: &mut Window) {
    let bounds = match shape {
        CursorShape::Beam => Bounds::new(origin, size(px(2.), line_height)),
        CursorShape::Underline => Bounds::new(point(origin.x, origin.y + line_height - px(2.)), size(cell_width, px(2.))),
        _ => Bounds::new(origin, size(cell_width, line_height)),
    };
    if focused {
        window.paint_quad(fill(bounds, color.opacity(0.6)));
    } else {
        window.paint_quad(outline(Bounds::new(origin, size(cell_width, line_height)), color, BorderStyle::Solid));
    }
}

fn cell_style(fg: Color, bg: Color, flags: Flags, colors: &Colors, selected: bool, palette: &Palette) -> CellStyle {
    let mut fg_rgb = palette.resolve(fg, colors, palette.foreground);
    let mut bg_rgb = palette.resolve(bg, colors, palette.background);
    if flags.contains(Flags::INVERSE) != selected {
        std::mem::swap(&mut fg_rgb, &mut bg_rgb);
    }
    let mut fg = to_hsla(fg_rgb);
    if flags.contains(Flags::DIM) {
        fg = fg.opacity(0.65);
    }
    CellStyle {
        fg,
        bg: (bg_rgb != palette.background).then(|| to_hsla(bg_rgb)),
        bold: flags.contains(Flags::BOLD),
        italic: flags.contains(Flags::ITALIC),
        underline: flags.intersects(Flags::ALL_UNDERLINES),
        strike: flags.contains(Flags::STRIKEOUT),
    }
}
