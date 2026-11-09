//! The element that paints a terminal frame.
//!
//! Painting a grid is not a layout problem: every cell is the same size, the
//! origin is the element's top-left corner, and nothing wraps. So this is one
//! `Element` that paints backgrounds, glyphs, decorations and the cursor, and
//! leaves layout to `window.request_layout`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

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
    /// A low-contrast foreground for chrome drawn inside the grid, such as the
    /// scrollbar. Using the caret color for this hid it against a pale grid.
    pub muted: Hsla,
    /// How strongly a selection tints a cell. The accent has to stay legible
    /// under text, and the two modes need opposite amounts of it.
    pub selection_alpha: f32,
    /// Whether the caret is in its visible blink phase. Ignored when the
    /// terminal asked for a steady cursor.
    pub cursor_visible: bool,
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
            muted: Hsla::default(),
            selection_alpha: 0.25,
            cursor_visible: true,
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
    /// Written during prepaint with where the grid was drawn in window
    /// coordinates: the IME anchors to it, the mouse resolves cells against it,
    /// and a split drag measures its extent.
    window_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The view that owns this grid, so the element can register itself as the
    /// platform's text input target while it paints.
    entity: Entity<Workspace>,
    focus: FocusHandle,
    /// Textures for images the terminal has drawn. Shared with every other
    /// grid in the window so a frame that has not changed is not re-uploaded.
    images: Rc<RefCell<ImageCache>>,
    /// Distinguishes this grid's images from another pane's in the shared cache.
    pane_id: u64,
}

impl TerminalGrid {
    /// Ten parameters is well past what the lint likes, and they are all
    /// needed: the grid has to reach the window, the workspace, the focus
    /// handle, the terminal its frame came from, and the cache its images are
    /// uploaded into. Each is a different kind of thing, and bundling them would
    /// move the spelling without making the call clearer.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        frame: Frame,
        style: GridStyle,
        focused: bool,
        dims: Rc<Cell<(u16, u16)>>,
        window_bounds: Rc<Cell<Bounds<Pixels>>>,
        entity: Entity<Workspace>,
        focus: FocusHandle,
        images: Rc<RefCell<ImageCache>>,
        pane_id: u64,
    ) -> Self {
        Self {
            id: id.into(),
            frame,
            style,
            focused,
            dims,
            window_bounds,
            entity,
            focus,
            images,
            pane_id,
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
        // The grid fills its container and nothing more. Requesting the cell
        // count as a minimum size would let a wide parser grid push the layout
        // past the window, which quietly put anything drawn at the bottom edge
        // - the scrollbar in particular - outside the visible area. `prepaint`
        // reports how many cells actually fit, and the owner resizes to that.
        let layout_id = window.request_layout(
            Style {
                size: gpui_kit::size(
                    Length::Definite(relative(1.0)),
                    Length::Definite(relative(1.0)),
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
        self.window_bounds.set(bounds);
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

        // The library defines three image layers and says to interleave them
        // with the renderer's own passes, bottom first. So the painting order is:
        // below-background images, cell backgrounds, below-text images, glyphs,
        // above-text images. Getting this wrong is not obvious from a screenshot:
        // an image in the wrong layer still looks correct until something covers
        // it.
        paint_images(
            window,
            bounds,
            style,
            &self.frame,
            self.pane_id,
            &self.images,
            vt::ImageLayer::BelowBackground,
        );

        // Pass one: cell backgrounds, so glyphs from a neighbouring cell never
        // sit on top of a background painted later.
        for (y, row) in rows.iter().enumerate() {
            for (x, cell) in row.cells.iter().take(fit_cols).enumerate() {
                let bg = resolve(cell.style.bg, &palette, default_bg);
                let bg = if cell.selected {
                    blend(bg, style.selection, style.selection_alpha)
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

        // Pass one and a half: images that sit under the text. They go after the
        // cell backgrounds, which would otherwise cover them, and before the
        // glyphs, which are meant to stay readable on top.
        paint_images(
            window,
            bounds,
            style,
            &self.frame,
            self.pane_id,
            &self.images,
            vt::ImageLayer::BelowText,
        );

        // Pass two: images that sit above the cell backgrounds.
        //
        // Every layer but the below-background one is drawn here, before the
        // glyphs, rather than after them. A layer is about what an image covers
        // when a terminal can lay out both itself; on Windows the console behind
        // the pseudo-terminal has no idea an image took any rows, so a line
        // editor that addresses its input line by absolute row keeps writing
        // into the rows the picture covers. Whatever we drew first is the thing
        // that has to give way, and the text is the thing people are reading.
        paint_images(
            window,
            bounds,
            style,
            &self.frame,
            self.pane_id,
            &self.images,
            vt::ImageLayer::BelowText,
        );
        paint_images(
            window,
            bounds,
            style,
            &self.frame,
            self.pane_id,
            &self.images,
            vt::ImageLayer::AboveText,
        );

        // Pass three: glyphs.
        for (y, row) in rows.iter().enumerate() {
            paint_row_glyphs(
                window, cx, row, y, fit_cols, origin, style, &palette, default_fg, default_bg,
                &font,
            );
        }

        // Pass four: decorations, drawn over the glyphs.
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

        // Pass five: the caret.
        let cursor = self.frame.cursor;
        if self.focused {
            // A blinking cursor is hidden on alternate phases; a steady one is
            // always drawn.
            let show =
                cursor.visible && cursor.in_viewport && (!cursor.blinking || style.cursor_visible);
            if show {
                let cell_box = cell_bounds(origin, style, cursor.x as usize, cursor.y as usize);
                paint_cursor(window, cx, cell_box, style, &self.frame, &font);
            }
        }

        paint_scrollbar(window, bounds, style, &self.frame);
    }
}

/// Draws a thin indicator while the viewport is scrolled back.
///
/// It only appears when there is history above or below, so a terminal showing
/// its live output stays free of chrome. The thumb is drawn as a pill rather
/// than a square bar: at three pixels wide a rectangular thumb reads as a
/// rendering artifact, while a rounded one reads as a control.
fn paint_scrollbar(window: &mut Window, bounds: Bounds<Pixels>, style: GridStyle, frame: &Frame) {
    let bar = frame.scrollbar;
    if bar.total <= bar.len || bar.len == 0 {
        return;
    }

    let width = px(4.0);
    let inset = px(2.0);
    let track_height = bounds.size.height - inset * 2.0;
    let thumb_height = (track_height * (bar.len as f32 / bar.total as f32))
        .max(px(24.0))
        .min(track_height);
    let max_offset = (bar.total - bar.len) as f32;
    let progress = if max_offset > 0.0 {
        (bar.offset as f32 / max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb_top = bounds.top() + inset + (track_height - thumb_height) * progress;

    // A faint track shows where the viewport sits in the whole buffer, which a
    // thumb alone cannot say when the history is long.
    window.paint_quad(quad(
        Bounds::new(
            point(bounds.right() - width - inset, bounds.top() + inset),
            size(width, track_height),
        ),
        Corners::all(width / 2.0),
        style.muted.opacity(0.12),
        px(0.0),
        transparent_black(),
        BorderStyle::default(),
    ));

    window.paint_quad(quad(
        Bounds::new(
            point(bounds.right() - width - inset, thumb_top),
            size(width, thumb_height),
        ),
        Corners::all(width / 2.0),
        style.muted.opacity(0.7),
        px(0.0),
        transparent_black(),
        BorderStyle::default(),
    ));
}

/// Draws the caret in the shape the terminal asked for.
///
/// The block style inverts the cell rather than covering it: the cursor color
/// goes down first, then the character is drawn again in the cell background
/// color. A solid block would hide the character under the caret, which is the
/// one cell the user is most likely to be reading.
#[allow(clippy::too_many_arguments)]
fn paint_cursor(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    style: GridStyle,
    frame: &Frame,
    font: &Font,
) {
    use vt::CursorStyle;

    let radius = px(1.5);
    let cell = frame
        .rows
        .get(frame.cursor.y as usize)
        .and_then(|row| row.cells.get(frame.cursor.x as usize));

    match frame.cursor.style {
        CursorStyle::Bar => {
            // Centred on the cell edge rather than flush against it, so the bar
            // reads as a caret instead of a divider between two characters.
            let width = px(2.0);
            let inset = (style.cell_width - width) / 2.0;
            window.paint_quad(quad(
                Bounds::new(
                    point(bounds.left() + inset, bounds.top()),
                    size(width, bounds.size.height),
                ),
                Corners::all(radius),
                style.cursor,
                px(0.0),
                transparent_black(),
                BorderStyle::default(),
            ));
        }
        CursorStyle::Underline => {
            let height = px(2.0);
            window.paint_quad(quad(
                Bounds::new(
                    point(bounds.left(), bounds.bottom() - height),
                    size(bounds.size.width, height),
                ),
                Corners::all(radius),
                style.cursor,
                px(0.0),
                transparent_black(),
                BorderStyle::default(),
            ));
        }
        CursorStyle::BlockHollow => {
            window.paint_quad(quad(
                bounds,
                Corners::all(radius),
                transparent_black(),
                px(1.0),
                style.cursor,
                BorderStyle::default(),
            ));
        }
        CursorStyle::Block => {
            window.paint_quad(quad(
                bounds,
                Corners::all(radius),
                style.cursor,
                px(0.0),
                transparent_black(),
                BorderStyle::default(),
            ));

            // The character under the caret, in the color the cell's background
            // would have been, so it stays readable on the filled block.
            if let Some(cell) = cell
                && !cell.text.is_empty()
                && !cell.style.invisible
            {
                paint_glyph(
                    window,
                    cx,
                    &cell.text,
                    style.background,
                    cell.style.bold,
                    bounds,
                    style,
                    font,
                );
            }
        }
    }
}

/// Draws every glyph of one row, in cell order.
///
/// Pulled out of the paint pass so the row's glyphs are written once, in one
/// place: the caret draws some of them a second time, and a second copy written
/// elsewhere would drift from this one.
#[allow(clippy::too_many_arguments)]
fn paint_row_glyphs(
    window: &mut Window,
    cx: &mut App,
    row: &vt::Row,
    y: usize,
    fit_cols: usize,
    origin: Point<Pixels>,
    style: GridStyle,
    palette: &[vt::Rgb; 256],
    default_fg: vt::Rgb,
    default_bg: vt::Rgb,
    font: &Font,
) {
    for (x, cell) in row.cells.iter().take(fit_cols).enumerate() {
        if cell.text.is_empty() || cell.style.invisible {
            continue;
        }

        let mut fg = resolve(cell.style.fg, palette, default_fg);
        if cell.style.inverse {
            fg = resolve(cell.style.bg, palette, default_bg);
        }
        if cell.style.faint {
            fg = blend(fg, style.background, 0.5);
        }

        let cell_box = cell_bounds(origin, style, x, y);
        paint_glyph(
            window,
            cx,
            &cell.text,
            fg,
            cell.style.bold,
            cell_box,
            style,
            font,
        );
    }
}

/// Shapes and paints one cell's text.
///
/// Extracted because the block cursor has to draw the same glyph a second time
/// in the inverted color. Leaving it inline meant a solid block sat on top of
/// the character and you could not read what the caret was over.
#[allow(clippy::too_many_arguments)]
fn paint_glyph(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    color: Hsla,
    bold: bool,
    cell_box: Bounds<Pixels>,
    style: GridStyle,
    font: &Font,
) {
    if text.is_empty() {
        return;
    }

    let run = TextRun {
        len: text.len(),
        font: Font {
            weight: if bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            ..font.clone()
        },
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };

    let line =
        window
            .text_system()
            .shape_line(text.to_owned().into(), style.font_size, &[run], None);

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
/// Lays `over` on `under` at the given opacity.
///
/// The alpha is a parameter because the two callers want different things: a
/// selection has to stay legible under text, while faint text only needs to be
/// nudged toward the background. A single hardcoded value served neither.
/// Lays `over` on `under` at the given opacity.
///
/// `Hsla::blend` puts its argument on top, so the order here matters: reading it
/// as `under.blend(over)` is what makes the selection visible at all. Getting it
/// backwards returns the background unchanged.
///
/// The alpha is a parameter because the two callers want different things: a
/// selection has to stay legible under text, while faint text only needs to be
/// nudged toward the background. A single hardcoded value served neither.
fn blend(under: Hsla, over: Hsla, alpha: f32) -> Hsla {
    under.blend(Hsla {
        h: over.h,
        s: over.s,
        l: over.l,
        a: alpha,
    })
}

/// Uploaded textures for the images a terminal has drawn.
///
/// Keyed by pane, image and generation. The generation is what makes the key
/// worth having: a program can send new pixels under the same image id, and the
/// library bumps the generation when it does, so a cache keyed on the id alone
/// would keep showing the old picture.
///
/// The cache lives in the workspace and is shared by every grid, because a grid
/// element is rebuilt each frame and anything it owned would be re-uploaded
/// from scratch every time.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<(u64, u32, u64), Arc<RenderImage>>,
    /// Keys the frame being painted has asked for. Everything else is dropped
    /// afterwards, so a terminal that has moved on from an image does not keep
    /// its texture alive.
    live: HashSet<(u64, u32, u64)>,
}

impl ImageCache {
    /// Starts a frame.
    ///
    /// Drops the textures the previous frame did not ask for, then clears the
    /// set of keys in use. Both halves happen here because painting is the only
    /// thing that fills the set, and there is no hook after it — so the set from
    /// the last paint is read at the start of the next one.
    ///
    /// Doing it in one place also means a frame that paints nothing at all, such
    /// as a window with no images anywhere, costs one empty retain.
    pub fn begin_frame(&mut self) {
        self.entries.retain(|key, _| self.live.contains(key));
        self.live.clear();
    }

    /// The texture for one placement, uploading the pixels if they are new.
    fn texture(
        &mut self,
        key: (u64, u32, u64),
        pixels: &[u8],
        width: u32,
        height: u32,
        bytes_per_pixel: usize,
    ) -> Option<Arc<RenderImage>> {
        self.live.insert(key);
        if let Some(found) = self.entries.get(&key) {
            return Some(found.clone());
        }

        let texture = upload(pixels, width, height, bytes_per_pixel)?;
        self.entries.insert(key, texture.clone());
        Some(texture)
    }
}

/// Turns stored pixels into a texture.
///
/// `RenderImage` wants BGRA, and the terminal stores RGB, RGBA or greyscale, so
/// the pixels are widened to four channels and the colour ones have their first
/// and last swapped. There is no decoding left to do: the library has already
/// turned PNG payloads into RGBA and inflated compressed ones.
fn upload(
    pixels: &[u8],
    width: u32,
    height: u32,
    bytes_per_pixel: usize,
) -> Option<Arc<RenderImage>> {
    if width == 0 || height == 0 {
        return None;
    }
    let expected = width as usize * height as usize * bytes_per_pixel;
    if pixels.len() < expected {
        return None;
    }

    let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
    for pixel in pixels[..expected].chunks_exact(bytes_per_pixel) {
        match bytes_per_pixel {
            4 => {
                bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
            3 => bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]),
            2 => bgra.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            _ => bgra.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
        }
    }

    // The buffer is typed RGBA but holds BGRA, which is what the renderer reads.
    // This is the same swap the framework's own image loader performs.
    let buffer = image::ImageBuffer::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// Draws the images one placement list asks for, in one layer.
///
/// The list is already sorted by layer and then z-index, so the caller paints
/// the two layers in two passes. A single pass would let an above-text image be
/// covered by a later below-text one.
fn paint_images(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    style: GridStyle,
    frame: &Frame,
    pane_id: u64,
    cache: &Rc<RefCell<ImageCache>>,
    layer: vt::ImageLayer,
) {
    let origin = bounds.origin;
    let mut cache = cache.borrow_mut();
    let mut painted: Vec<PendingImage> = Vec::new();

    for entry in frame
        .images
        .iter()
        .filter(|entry| entry.placement.layer == layer)
    {
        let placement = &entry.placement;
        let image = &entry.image;

        let key = (pane_id, placement.image_id, placement.generation);
        let texture = match cache.texture(
            key,
            &image.pixels,
            image.width,
            image.height,
            image.format.bytes_per_pixel(),
        ) {
            Some(texture) => texture,
            None => continue,
        };

        // The rectangle the placement occupies, in this renderer's own cell
        // metrics. The library also reports a pixel width and height, computed
        // from the cell size *it* was told; the two agree as long as the
        // terminal is told the size measured here, which `Session::resize` does.
        let cell_left = origin.x + style.cell_width * placement.col as f32;
        let cell_top = origin.y + style.cell_height * placement.row as f32;
        let cell_rect = Bounds::new(
            point(
                // A program may ask for a sub-cell offset; it is always under one
                // cell, so it only nudges the image inside its rectangle.
                cell_left + px(placement.offset_x as f32),
                cell_top + px(placement.offset_y as f32),
            ),
            size(
                style.cell_width * placement.cols.max(1) as f32,
                style.cell_height * placement.rows.max(1) as f32,
            ),
        );

        // A placement can show a sub-rectangle of its image. Scale the whole
        // image as if it filled the rectangle, then shift it so the requested
        // part lands there; the clip trims the rest.
        //
        // The rectangle's shape is not corrected. A program decides both the
        // cells and the part of the image to show, and it bakes its aspect into
        // the source rectangle — `chafa` scales a 128×128 icon to 128×138 before
        // sending it. Stretching into the declared rectangle is what Ghostty and
        // Kero both do; fitting the image's own aspect here would fight the
        // program that already did.
        let source_w = placement.source_width.max(1) as f32;
        let source_h = placement.source_height.max(1) as f32;
        let scale_x = cell_rect.size.width.as_f32() / source_w;
        let scale_y = cell_rect.size.height.as_f32() / source_h;
        let image_bounds = Bounds::new(
            point(
                cell_rect.origin.x - px(placement.source_x as f32 * scale_x),
                cell_rect.origin.y - px(placement.source_y as f32 * scale_y),
            ),
            size(
                px(image.width as f32 * scale_x),
                px(image.height as f32 * scale_y),
            ),
        );

        // Clipped to both the cell rectangle and the pane, so a placement that
        // runs past either is cut off rather than spilling over the text or the
        // neighbouring pane.
        let clip = cell_rect.intersect(&bounds);
        if clip.size.width <= px(0.0) || clip.size.height <= px(0.0) {
            continue;
        }
        painted.push((
            layer_rank(placement.layer),
            placement.z,
            texture,
            clip,
            image_bounds,
        ));
    }

    // Sorted here rather than trusted: the frame is built from the render state
    // in whatever order the library reports, and a higher z has to land on top.
    painted.sort_by_key(|(rank, z, ..)| (*rank, *z));
    for (_, _, texture, clip, image_bounds) in painted {
        let _ = window.paint_image(clip, image_bounds, Corners::default(), texture, 0, false);
    }
}

/// One image ready to paint: its draw order, texture, clip and destination.
type PendingImage = (u8, i32, Arc<RenderImage>, Bounds<Pixels>, Bounds<Pixels>);

/// Draw order of the layers, bottom first.
fn layer_rank(layer: vt::ImageLayer) -> u8 {
    match layer {
        vt::ImageLayer::BelowBackground => 0,
        vt::ImageLayer::BelowText => 1,
        vt::ImageLayer::AboveText => 2,
    }
}
