//! The element that paints a terminal frame.
//!
//! Painting a grid is not a layout problem: every cell is the same size, the
//! origin is the element's top-left corner, and nothing wraps. So this is one
//! `Element` that paints backgrounds, glyphs, decorations and the cursor, and
//! leaves layout to `window.request_layout`.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::*;

use crate::Workspace;
use crate::terminal::Frame;

/// How the grid is drawn: geometry plus the colors to use.
#[derive(Clone, Copy, Debug)]
pub struct GridStyle {
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub font_size: Pixels,
    pub baseline: Pixels,
    pub background: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
}

impl Default for GridStyle {
    /// A placeholder used before the first frame measures the font.
    fn default() -> Self {
        Self {
            cell_width: px(8.0),
            cell_height: px(18.0),
            font_size: px(14.0),
            baseline: px(14.0),
            background: Hsla::default(),
            cursor: Hsla::default(),
            selection: Hsla::default(),
        }
    }
}

/// Paints one terminal frame into the bounds it is given.
pub struct TerminalGrid {
    id: ElementId,
    frame: Frame,
    style: GridStyle,
    focused: bool,
    /// Written during prepaint with the cell count the element actually has
    /// room for, so the owner can size the pseudo console to match.
    dims: Rc<Cell<(u16, u16)>>,
    /// Written during prepaint with the grid's position in window coordinates,
    /// so the IME can anchor its candidate window to the caret.
    window_origin: Rc<Cell<Point<Pixels>>>,
    /// The view that owns this grid, so the element can register itself as the
    /// platform's text input target while it paints.
    entity: Entity<Workspace>,
    focus: FocusHandle,
}

impl TerminalGrid {
    pub fn new(
        id: impl Into<ElementId>,
        frame: Frame,
        style: GridStyle,
        focused: bool,
        dims: Rc<Cell<(u16, u16)>>,
        window_origin: Rc<Cell<Point<Pixels>>>,
        entity: Entity<Workspace>,
        focus: FocusHandle,
    ) -> Self {
        Self {
            id: id.into(),
            frame,
            style,
            focused,
            dims,
            window_origin,
            entity,
            focus,
        }
    }
}

impl IntoElement for TerminalGrid {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalGrid {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let cols = self.frame.rows.first().map_or(0, |row| row.cells.len());
        let rows = self.frame.rows.len();
        let width = self.style.cell_width * cols as f32;
        let height = self.style.cell_height * rows as f32;

        let layout_id = window.request_layout(
            Style {
                size: gpui_kit::size(
                    Length::Definite(relative(1.0)),
                    Length::Definite(relative(1.0)),
                ),
                min_size: gpui_kit::size(
                    Length::Definite(width.into()),
                    Length::Definite(height.into()),
                ),
                ..Default::default()
            },
            [],
            cx,
        );
        (layout_id, ())
    }
    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        // The element is sized by its container, so this is the only place that
        // knows how many whole cells actually fit. Reporting it here rather than
        // guessing from the window keeps the grid inside its pane.
        let cols = (bounds.size.width.as_f32() / self.style.cell_width.as_f32())
            .floor()
            .max(1.0);
        let rows = (bounds.size.height.as_f32() / self.style.cell_height.as_f32())
            .floor()
            .max(1.0);
        self.dims.set((cols as u16, rows as u16));
        self.window_origin.set(bounds.origin);
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let style = self.style;

        // Registering the grid as the text input target is what lets an IME
        // reach the shell at all. The platform asks the focused handler for
        // caret geometry and delivers the committed text back through it.
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.entity.clone()),
            cx,
        );

        window.paint_quad(fill(bounds, style.background));

        let origin = bounds.origin;
        let palette = self.frame.palette;
        let default_fg = self.frame.foreground;
        let default_bg = self.frame.background;
        let font_family = gpui_kit::component::Theme::global(cx)
            .mono_font_family
            .clone();
        let font = Font {
            family: font_family,
            weight: FontWeight::NORMAL,
            ..Default::default()
        };

        // Only the cells that fit are drawn. The parser may still hold a wider
        // grid for a frame after a resize, and painting past the bounds would
        // spill into the neighbouring pane.
        let fit_cols = (bounds.size.width.as_f32() / style.cell_width.as_f32())
            .floor()
            .max(0.0) as usize;
        let fit_rows = (bounds.size.height.as_f32() / style.cell_height.as_f32())
            .floor()
            .max(0.0) as usize;
        let rows: Vec<&vt::Row> = self.frame.rows.iter().take(fit_rows).collect();

        // Pass one: cell backgrounds, so glyphs from a neighbouring cell never
        // sit on top of a background painted later.
        for (y, row) in rows.iter().enumerate() {
            for (x, cell) in row.cells.iter().take(fit_cols).enumerate() {
                let bg = resolve(cell.style.bg, &palette, default_bg);
                let bg = if cell.selected {
                    blend(bg, style.selection)
                } else if cell.style.inverse {
                    resolve(cell.style.fg, &palette, default_fg)
                } else {
                    bg
                };

                if bg != style.background || cell.selected || cell.style.inverse {
                    window.paint_quad(fill(cell_bounds(origin, style, x, y), bg));
                }
            }
        }

        // Pass two: glyphs.
        for (y, row) in rows.iter().enumerate() {
            for (x, cell) in row.cells.iter().take(fit_cols).enumerate() {
                if cell.text.is_empty() || cell.style.invisible {
                    continue;
                }

                let mut fg = resolve(cell.style.fg, &palette, default_fg);
                if cell.style.inverse {
                    fg = resolve(cell.style.bg, &palette, default_bg);
                }
                if cell.style.faint {
                    fg = blend(fg, style.background);
                }

                let run = TextRun {
                    len: cell.text.len(),
                    font: Font {
                        weight: if cell.style.bold {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        },
                        ..font.clone()
                    },
                    color: fg,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };

                let line = window.text_system().shape_line(
                    cell.text.clone().into(),
                    style.font_size,
                    &[run],
                    None,
                );

                let cell_box = cell_bounds(origin, style, x, y);
                let glyph_origin = point(
                    cell_box.left(),
                    cell_box.top() + style.baseline - line.ascent,
                );

                let _ = line.paint(
                    glyph_origin,
                    style.cell_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }

        // Pass three: decorations and the cursor, drawn over the glyphs.
        for (y, row) in rows.iter().enumerate() {
            for (x, cell) in row.cells.iter().take(fit_cols).enumerate() {
                let cell_box = cell_bounds(origin, style, x, y);
                let mut fg = resolve(cell.style.fg, &palette, default_fg);
                if cell.style.inverse {
                    fg = resolve(cell.style.bg, &palette, default_bg);
                }

                if cell.style.underline {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(cell_box.left(), cell_box.bottom() - px(1.0)),
                            size(cell_box.size.width, px(1.0)),
                        ),
                        fg,
                    ));
                }
                if cell.style.strikethrough {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(cell_box.left(), cell_box.top() + style.cell_height * 0.55),
                            size(cell_box.size.width, px(1.0)),
                        ),
                        fg,
                    ));
                }
                if cell.style.overline {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(cell_box.left(), cell_box.top()),
                            size(cell_box.size.width, px(1.0)),
                        ),
                        fg,
                    ));
                }
            }
        }

        if self.focused {
            let cursor = self.frame.cursor;
            if cursor.visible && cursor.in_viewport {
                let cell_box = cell_bounds(origin, style, cursor.x as usize, cursor.y as usize);
                paint_cursor(window, cell_box, style, &self.frame);
            }
        }

        paint_scrollbar(window, bounds, style, &self.frame);
    }
}

/// Draws a thin indicator while the viewport is scrolled back.
///
/// It only appears when there is history above or below, so a terminal showing
/// its live output stays free of chrome.
fn paint_scrollbar(window: &mut Window, bounds: Bounds<Pixels>, style: GridStyle, frame: &Frame) {
    let bar = frame.scrollbar;
    if bar.total <= bar.len || bar.len == 0 {
        return;
    }

    let width = px(3.0);
    let inset = px(2.0);
    let track_height = bounds.size.height - inset * 2.0;
    let thumb_height = (track_height * (bar.len as f32 / bar.total as f32))
        .max(px(16.0))
        .min(track_height);
    let max_offset = (bar.total - bar.len) as f32;
    let progress = if max_offset > 0.0 {
        (bar.offset as f32 / max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb_top = bounds.top() + inset + (track_height - thumb_height) * progress;

    window.paint_quad(fill(
        Bounds::new(
            point(bounds.right() - width - inset, thumb_top),
            size(width, thumb_height),
        ),
        style.cursor.opacity(0.35),
    ));
}

fn paint_cursor(window: &mut Window, bounds: Bounds<Pixels>, style: GridStyle, frame: &Frame) {
    use vt::CursorStyle;

    // A block cursor inverts the cell: paint the cursor color behind and the
    // cell's own text over it. Drawing the glyph again is what makes the
    // character readable instead of a solid block.
    match frame.cursor.style {
        CursorStyle::Bar => {
            window.paint_quad(fill(
                Bounds::new(bounds.origin, size(px(2.0), bounds.size.height)),
                style.cursor,
            ));
        }
        CursorStyle::Underline => {
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.left(), bounds.bottom() - px(2.0)),
                    size(bounds.size.width, px(2.0)),
                ),
                style.cursor,
            ));
        }
        CursorStyle::BlockHollow => {
            window.paint_quad(quad(
                bounds,
                Corners::default(),
                transparent_black(),
                px(1.0),
                style.cursor,
                BorderStyle::default(),
            ));
        }
        CursorStyle::Block => {
            window.paint_quad(fill(bounds, style.cursor));
        }
    }
}

fn cell_bounds(origin: Point<Pixels>, style: GridStyle, x: usize, y: usize) -> Bounds<Pixels> {
    Bounds::new(
        point(
            origin.x + style.cell_width * x as f32,
            origin.y + style.cell_height * y as f32,
        ),
        size(style.cell_width, style.cell_height),
    )
}

/// Resolves a cell color against the terminal's palette.
///
/// The library already resolves colors, so a zeroed color means "unset" and the
/// default applies.
fn resolve(color: vt::Rgb, palette: &[vt::Rgb; 256], default: vt::Rgb) -> Hsla {
    let _ = palette;
    if color == vt::Rgb::default() {
        rgb(default)
    } else {
        rgb(color)
    }
}

fn rgb(color: vt::Rgb) -> Hsla {
    gpui_kit::rgb(u32::from_be_bytes([0, color.r, color.g, color.b])).into()
}

/// Lays `over` on top of `under` at half strength, for faint text and
/// selection tint.
fn blend(under: Hsla, over: Hsla) -> Hsla {
    Hsla {
        h: over.h,
        s: over.s,
        l: over.l,
        a: 0.55,
    }
    .blend(under)
}
