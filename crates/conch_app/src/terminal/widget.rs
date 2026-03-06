//! Terminal rendering via egui's `Painter` API.
//!
//! The core rendering loop iterates over `Term::renderable_content().display_iter`,
//! painting cell backgrounds and characters with `rect_filled` and `galley`.
//! This replaces the wgpu shader pipeline from the old `conch_terminal` crate.

use std::sync::Arc;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::term::Term;
use conch_session::EventProxy;
use egui::{Color32, FontFamily, FontId, Painter, Pos2, Rect, Sense, Vec2};

use super::color::{convert_color, ResolvedColors};
use super::size_info::SizeInfo;

/// Convert an `[f32; 4]` RGBA color to egui's `Color32`.
#[inline]
fn rgba_to_color32(c: [f32; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
        (c[3] * 255.0) as u8,
    )
}

/// Measure the monospace font's cell dimensions from egui's layout engine.
///
/// Uses differential measurement -- `width(10 chars) - width(1 char)` divided by 9 --
/// to eliminate any fixed side-bearing overhead in galley sizes.
pub fn measure_cell_size(ctx: &egui::Context, font_size: f32) -> (f32, f32) {
    let font_id = FontId::new(font_size, FontFamily::Monospace);
    ctx.fonts(|fonts| {
        let g1 = fonts.layout_no_wrap("M".to_string(), font_id.clone(), Color32::WHITE);
        let g10 = fonts.layout_no_wrap("MMMMMMMMMM".to_string(), font_id, Color32::WHITE);
        let width = (g10.size().x - g1.size().x) / 9.0;
        let height = g1.size().y;
        (width, height)
    })
}

/// Convert a pixel position (relative to the window) to a terminal cell `(col, row)`.
pub fn pixel_to_cell(pos: Pos2, rect_min: Pos2, size_info: &SizeInfo) -> (usize, usize) {
    let x = (pos.x - rect_min.x - size_info.padding_x).max(0.0);
    let y = (pos.y - rect_min.y - size_info.padding_y).max(0.0);
    let col = (x / size_info.cell_width) as usize;
    let row = (y / size_info.cell_height) as usize;
    (col, row)
}

/// Check whether cell `(col, row)` falls within the normalized selection range.
#[inline]
fn is_in_selection(col: usize, row: usize, start: (usize, usize), end: (usize, usize)) -> bool {
    if row < start.1 || row > end.1 {
        return false;
    }
    if start.1 == end.1 {
        return col >= start.0 && col <= end.0;
    }
    if row == start.1 {
        return col >= start.0;
    }
    if row == end.1 {
        return col <= end.0;
    }
    true
}

/// Copied cell data for rendering after releasing the terminal lock.
struct CellInfo {
    c: char,
    col: usize,
    row: usize,
    fg: [f32; 4],
    bg: [f32; 4],
    underline: bool,
    strikeout: bool,
}

/// Paint the terminal grid into the given UI region.
///
/// Returns the `Response` (for mouse interaction) and the computed `SizeInfo`.
pub fn show_terminal(
    ui: &mut egui::Ui,
    term: &Arc<FairMutex<Term<EventProxy>>>,
    cell_width: f32,
    cell_height: f32,
    colors: &ResolvedColors,
    font_size: f32,
    cursor_visible: bool,
    selection: Option<((usize, usize), (usize, usize))>,
) -> (egui::Response, SizeInfo) {
    let available = ui.available_size();
    let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
    let rect = response.rect;

    let size_info = SizeInfo::new(rect.width(), rect.height(), cell_width, cell_height);

    // Fill the entire allocation with the terminal background.
    painter.rect_filled(rect, 0.0, rgba_to_color32(colors.background));

    // ── Collect cell data under lock, then release ──────────────────────
    // This minimises FairMutex hold time so the EventLoop can keep
    // processing VTE data while we do the (expensive) font-layout paint.
    let (cells, cursor_pos) = {
        let term = term.lock();
        let content = term.renderable_content();

        let show_cursor = cursor_visible
            && selection.is_none()
            && content
                .mode
                .contains(alacritty_terminal::term::TermMode::SHOW_CURSOR);

        let cursor_pos = if show_cursor {
            Some((content.cursor.point.column.0, content.cursor.point.line.0 as usize, content.cursor.shape))
        } else {
            None
        };

        let mut cells = Vec::with_capacity(size_info.columns() * size_info.rows());
        for indexed in content.display_iter {
            let cell = indexed.cell;
            let point = indexed.point;
            let flags = cell.flags;

            if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            let mut fg = convert_color(cell.fg, colors);
            let mut bg = convert_color(cell.bg, colors);

            if flags.contains(CellFlags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }

            let col = point.column.0;
            let row = point.line.0 as usize;

            if let Some((sel_start, sel_end)) = selection {
                if is_in_selection(col, row, sel_start, sel_end) {
                    if let (Some(sel_bg), Some(sel_fg)) = (colors.selection_bg, colors.selection_text) {
                        bg = sel_bg;
                        fg = sel_fg;
                    } else {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                }
            }

            cells.push(CellInfo {
                c: cell.c,
                col,
                row,
                fg,
                bg,
                underline: flags.contains(CellFlags::UNDERLINE),
                strikeout: flags.contains(CellFlags::STRIKEOUT),
            });
        }

        (cells, cursor_pos)
    }; // ── lock released here ──────────────────────────────────────────────

    // Paint cells (no lock held — EventLoop can process data concurrently).
    let font_id = FontId::new(font_size, FontFamily::Monospace);

    for ci in &cells {
        let (x, y) = size_info.cell_position(ci.col, ci.row);

        if ci.bg != colors.background {
            let cell_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + x, rect.min.y + y),
                Vec2::new(cell_width, cell_height),
            );
            painter.rect_filled(cell_rect, 0.0, rgba_to_color32(ci.bg));
        }

        if ci.c != ' ' && ci.c != '\0' {
            paint_char(
                &painter,
                ci.c,
                Pos2::new(rect.min.x + x, rect.min.y + y),
                &font_id,
                rgba_to_color32(ci.fg),
                ci.underline,
                ci.strikeout,
                cell_width,
                cell_height,
            );
        }
    }

    // Draw cursor (Block, Underline, or Beam).
    if let Some((col, row, shape)) = cursor_pos {
        let (cx, cy) = size_info.cell_position(col, row);
        let cursor_c = colors.cursor_color.unwrap_or(colors.foreground);
        let color = rgba_to_color32(cursor_c);
        match shape {
            alacritty_terminal::vte::ansi::CursorShape::Block => {
                let cursor_rect = Rect::from_min_size(
                    Pos2::new(rect.min.x + cx, rect.min.y + cy),
                    Vec2::new(cell_width, cell_height),
                );
                painter.rect_filled(cursor_rect, 0.0, color);
            }
            alacritty_terminal::vte::ansi::CursorShape::Underline => {
                let thickness = (cell_height * 0.1).max(1.0);
                let cursor_rect = Rect::from_min_size(
                    Pos2::new(rect.min.x + cx, rect.min.y + cy + cell_height - thickness),
                    Vec2::new(cell_width, thickness),
                );
                painter.rect_filled(cursor_rect, 0.0, color);
            }
            alacritty_terminal::vte::ansi::CursorShape::Beam => {
                let thickness = (cell_width * 0.12).max(1.0);
                let cursor_rect = Rect::from_min_size(
                    Pos2::new(rect.min.x + cx, rect.min.y + cy),
                    Vec2::new(thickness, cell_height),
                );
                painter.rect_filled(cursor_rect, 0.0, color);
            }
            alacritty_terminal::vte::ansi::CursorShape::HollowBlock => {
                let cursor_rect = Rect::from_min_size(
                    Pos2::new(rect.min.x + cx, rect.min.y + cy),
                    Vec2::new(cell_width, cell_height),
                );
                painter.rect_stroke(cursor_rect, 0.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
            alacritty_terminal::vte::ansi::CursorShape::Hidden => {}
        }
    }

    (response, size_info)
}

/// Extract the text within a normalized selection range from the terminal buffer.
pub fn get_selected_text(
    term: &Arc<FairMutex<Term<EventProxy>>>,
    sel_start: (usize, usize),
    sel_end: (usize, usize),
) -> String {
    let term = term.lock();
    let content = term.renderable_content();

    let mut lines: Vec<String> = Vec::new();
    let mut current_row: Option<usize> = None;
    let mut current_line = String::new();

    for indexed in content.display_iter {
        let col = indexed.point.column.0;
        let row = indexed.point.line.0 as usize;

        if row > sel_end.1 {
            break;
        }
        if !is_in_selection(col, row, sel_start, sel_end) {
            continue;
        }

        if current_row != Some(row) {
            if current_row.is_some() {
                lines.push(current_line.trim_end().to_string());
                current_line = String::new();
            }
            current_row = Some(row);
        }

        current_line.push(indexed.cell.c);
    }

    if !current_line.is_empty() {
        lines.push(current_line.trim_end().to_string());
    }

    lines.join("\n")
}

/// Render a single character centered in its cell, with optional underline/strikeout.
fn paint_char(
    painter: &Painter,
    c: char,
    pos: Pos2,
    font_id: &FontId,
    color: Color32,
    underline: bool,
    strikeout: bool,
    cell_width: f32,
    cell_height: f32,
) {
    let galley = painter.layout_no_wrap(c.to_string(), font_id.clone(), color);
    let text_width = galley.size().x;
    let offset_x = (cell_width - text_width) / 2.0;
    painter.galley(Pos2::new(pos.x + offset_x, pos.y), galley, color);

    if underline {
        let y = pos.y + cell_height - 1.0;
        painter.line_segment(
            [Pos2::new(pos.x, y), Pos2::new(pos.x + cell_width, y)],
            egui::Stroke::new(1.0, color),
        );
    }

    if strikeout {
        let y = pos.y + cell_height * 0.5;
        painter.line_segment(
            [Pos2::new(pos.x, y), Pos2::new(pos.x + cell_width, y)],
            egui::Stroke::new(1.0, color),
        );
    }
}
