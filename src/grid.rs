//! Turning a [`Frame`] into a [`Painter`] drawing.

use nsh_core::canvas::CanvasTextBaseline;
use nsh_core::prelude::*;

use crate::term::{CellData, Frame};

#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub cell_w: f32,
    pub cell_h: f32,
    pub pad: f32,
    pub font: String,
}

impl Metrics {
    pub fn new(font_size: f32) -> Self {
        let font = format!("{font_size}px \"Cascadia Mono\"");
        let measured = text_metrics(&font, "M");
        Self {
            cell_w: measured.width.0.max(1.0),
            cell_h: (font_size * 1.25).round().max(1.0),
            pad: 8.0,
            font,
        }
    }

    pub fn cols(&self, width: f32) -> u16 {
        (((width - 2.0 * self.pad) / self.cell_w).floor().max(1.0)) as u16
    }

    pub fn rows(&self, height: f32) -> u16 {
        (((height - 2.0 * self.pad) / self.cell_h).floor().max(1.0)) as u16
    }
}

fn hsla(color: [u8; 3]) -> Hsla {
    let packed = (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2]);
    rgb(packed).into()
}

fn cell_at(frame: &Frame, row: usize, col: usize) -> &CellData {
    &frame.cells[row * usize::from(frame.cols) + col]
}

/// Draws one viewport: background runs, then text, then the cursor.
pub fn draw(frame: &Frame, metrics: &Metrics) -> Painter {
    let mut painter = Painter::new()
        .font(&metrics.font)
        .text_baseline(CanvasTextBaseline::Top)
        .text_align(nsh_core::canvas::CanvasTextAlign::Left);

    // Background: merge equal neighbours into one rectangle per run.
    for row in 0..usize::from(frame.rows) {
        let y = metrics.pad + row as f32 * metrics.cell_h;
        let mut run: Option<([u8; 3], usize)> = None;
        for col in 0..=usize::from(frame.cols) {
            let current = (col < usize::from(frame.cols))
                .then(|| cell_at(frame, row, col))
                .filter(|cell| cell.has_bg)
                .map(|cell| cell.bg);
            match (run, current) {
                (Some((color, start)), Some(next)) if color == next => {
                    run = Some((color, start));
                }
                (Some((color, start)), _) => {
                    let width = (col - start) as f32 * metrics.cell_w;
                    painter = painter.fill_color(hsla(color)).fill_rect(
                        metrics.pad + start as f32 * metrics.cell_w,
                        y,
                        width,
                        metrics.cell_h,
                    );
                    run = current.map(|color| (color, col));
                }
                (None, Some(next)) => run = Some((next, col)),
                (None, None) => {}
            }
        }
    }

    // Selection: one rectangle per run of selected cells.
    for row in 0..usize::from(frame.rows) {
        let y = metrics.pad + row as f32 * metrics.cell_h;
        let mut run: Option<usize> = None;
        for col in 0..=usize::from(frame.cols) {
            let selected = col < usize::from(frame.cols) && cell_at(frame, row, col).selected;
            match (run, selected) {
                (Some(start), true) => run = Some(start),
                (Some(start), false) => {
                    let width = (col - start) as f32 * metrics.cell_w;
                    painter = painter.fill_color(hsla([0x2b, 0x4a, 0x7a])).fill_rect(
                        metrics.pad + start as f32 * metrics.cell_w,
                        y,
                        width,
                        metrics.cell_h,
                    );
                    run = None;
                }
                (None, true) => run = Some(col),
                (None, false) => {}
            }
        }
    }

    // Text and its decorations.
    for row in 0..usize::from(frame.rows) {
        let y = metrics.pad + row as f32 * metrics.cell_h;
        for col in 0..usize::from(frame.cols) {
            let cell = cell_at(frame, row, col);
            if cell.text.is_empty() {
                continue;
            }
            let x = metrics.pad + col as f32 * metrics.cell_w;
            painter = painter
                .fill_color(hsla(cell.fg))
                .fill_text(&cell.text, x, y);
            if cell.underline {
                painter = painter.fill_color(hsla(cell.fg)).fill_rect(
                    x,
                    y + metrics.cell_h - 1.0,
                    metrics.cell_w,
                    1.0,
                );
            }
        }
    }

    // Cursor: a block over its cell, its glyph drawn in the background color.
    if frame.cursor_visible {
        if let Some((cx, cy)) = frame.cursor {
            if cx < frame.cols && cy < frame.rows {
                let x = metrics.pad + f32::from(cx) * metrics.cell_w;
                let y = metrics.pad + f32::from(cy) * metrics.cell_h;
                let cell = cell_at(frame, usize::from(cy), usize::from(cx));
                painter = painter.fill_color(hsla(frame.foreground)).fill_rect(
                    x,
                    y,
                    metrics.cell_w,
                    metrics.cell_h,
                );
                if !cell.text.is_empty() {
                    painter = painter
                        .fill_color(hsla(frame.background))
                        .fill_text(&cell.text, x, y);
                }
            }
        }
    }

    painter
}
