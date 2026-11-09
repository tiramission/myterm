//! Images a program has drawn into the terminal with the Kitty graphics
//! protocol.
//!
//! The library keeps two halves apart, and so does this module. The
//! *placements* come from the render state and say where on screen an image
//! goes; the *pixels* come from the terminal's image storage and say what to
//! draw there. A renderer needs both, and neither is enough alone: a placement
//! without its image is a hole in the text, and an image without a placement is
//! a picture nobody asked to see.
//!
//! Both halves borrow from the library and are copied out here. Placement data
//! is only valid until the next render-state update, and pixel data only until
//! the terminal is next modified, so anything that crosses that boundary has to
//! be owned.

use std::ffi::c_void;

use vt_sys as sys;

use crate::Result;

/// The layer an image is drawn in.
///
/// The library defines these in draw order, bottom first, and a renderer has to
/// interleave them with its own passes: below-background, cell backgrounds,
/// below-text, text, above-text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageLayer {
    /// Beneath the cell background colours. A Kitty placement lands here when
    /// its z-index is below `INT32_MIN / 2`.
    BelowBackground,
    /// Above the cell backgrounds and below the text, so the text stays
    /// readable over it. Kitty z-index from `INT32_MIN / 2` up to `-1`.
    BelowText,
    /// Above the text, which is what a program asks for when it means to cover
    /// what is already on screen. Kitty z-index 0 and above.
    AboveText,
}

/// Pixel format of a stored image.
///
/// PNG never appears: payloads are decoded to RGBA before they are stored, so a
/// consumer never has to decode anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Rgb,
    Rgba,
    GrayAlpha,
    Gray,
}

impl ImageFormat {
    fn from_sys(value: i32) -> Option<Self> {
        match value {
            sys::GHOSTTY_KITTY_IMAGE_FORMAT_RGB => Some(Self::Rgb),
            sys::GHOSTTY_KITTY_IMAGE_FORMAT_RGBA => Some(Self::Rgba),
            sys::GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA => Some(Self::GrayAlpha),
            sys::GHOSTTY_KITTY_IMAGE_FORMAT_GRAY => Some(Self::Gray),
            _ => None,
        }
    }

    /// Bytes per pixel in this format.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgb => 3,
            Self::Rgba => 4,
            Self::GrayAlpha => 2,
            Self::Gray => 1,
        }
    }
}

/// Where an image goes, and which stored image to put there.
///
/// Everything is in the render state's own coordinates: cells are viewport
/// cells with `(0, 0)` at the top left of the visible grid, and pixels are
/// terminal pixels. `col` and `row` can be negative — a fixed or relative
/// placement is reported at its full size even when it starts off screen.
#[derive(Clone, Debug)]
pub struct ImagePlacement {
    /// Which stored image to draw.
    pub image_id: u32,
    /// Changes whenever the image's pixels change, including when an animation
    /// moves to a new frame. Part of a texture cache key.
    pub generation: u64,
    pub layer: ImageLayer,
    /// Orders images within a layer; lower draws first.
    pub z: i32,
    pub col: i32,
    pub row: i32,
    /// How many cells the placement covers.
    pub cols: u32,
    pub rows: u32,
    /// Where the image starts inside its top-left cell, in pixels.
    pub offset_x: u32,
    pub offset_y: u32,
    /// The size to draw at, in pixels.
    pub width: u32,
    pub height: u32,
    /// The part of the image to draw, in image pixels. Scale this to `width`
    /// and `height`; it is usually the whole image.
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
}

/// A decoded image out of the terminal's storage.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    /// Tightly packed pixels, `width * height * bytes_per_pixel` long.
    pub pixels: Vec<u8>,
    /// Changes whenever the pixels change. The render state reports the same
    /// value for each placement of this image.
    pub generation: u64,
}

impl Image {
    /// Reads one image out of the terminal's Kitty graphics storage.
    ///
    /// `None` when there is no image with that id, or when its pixels have not
    /// finished arriving — the render state skips those placements, so a
    /// renderer only ever asks for images that are ready.
    pub(crate) fn from_storage(storage: sys::GhosttyKittyGraphics, image_id: u32) -> Option<Self> {
        // SAFETY: a null handle yields a null image, which is checked below.
        // Every field is read into a local of the documented type.
        unsafe {
            let raw = sys::ghostty_kitty_graphics_image(storage, image_id);
            if raw.is_null() {
                return None;
            }

            let mut width = 0u32;
            let mut height = 0u32;
            let mut format = 0i32;
            let mut generation = 0u64;
            let mut pixels = sys::GhosttyString {
                ptr: std::ptr::null(),
                len: 0,
            };

            // The library reports a status code rather than a `Result`, so each
            // read is checked against success here rather than with `?`.
            macro_rules! read {
                ($key:expr, $out:expr) => {
                    sys::ghostty_kitty_graphics_image_get(raw, $key, $out as *mut _ as *mut c_void)
                        == sys::GHOSTTY_SUCCESS
                };
            }

            let sized = read!(sys::GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_WIDTH, &mut width)
                && read!(sys::GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_HEIGHT, &mut height)
                && read!(sys::GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_FORMAT, &mut format)
                && read!(
                    sys::GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_GENERATION,
                    &mut generation
                );
            if !sized {
                return None;
            }

            let format = ImageFormat::from_sys(format)?;

            // The pointer belongs to the terminal and is only valid until it is
            // next modified, so the pixels are copied out before returning.
            if !read!(sys::GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_PIXELS, &mut pixels)
                || pixels.ptr.is_null()
            {
                return None;
            }
            let bytes = std::slice::from_raw_parts(pixels.ptr, pixels.len);

            Some(Self {
                width,
                height,
                format,
                pixels: bytes.to_vec(),
                generation,
            })
        }
    }
}

/// An iterator over the image placements a render state reports.
///
/// Created once and refilled from each render-state update; the placements it
/// walks are only valid until the next one.
pub(crate) struct ImageIterator {
    raw: sys::GhosttyRenderStateImageIterator,
}

impl ImageIterator {
    pub(crate) fn new() -> Result<Self> {
        let mut raw: sys::GhosttyRenderStateImageIterator = std::ptr::null_mut();
        // SAFETY: the out pointer is a local of the right type, and the null
        // allocator asks the library to use its own.
        crate::check(unsafe {
            sys::ghostty_render_state_image_iterator_new(std::ptr::null(), &mut raw)
        })?;
        Ok(Self { raw })
    }

    /// Fills the iterator from a render state and walks every placement.
    ///
    /// The order the library reports is not the order to draw in: the result is
    /// sorted by layer and then by z-index, which is the order a renderer wants.
    pub(crate) fn placements(
        &mut self,
        state: sys::GhosttyRenderState,
    ) -> Result<Vec<ImagePlacement>> {
        // SAFETY: the state handle is the one this crate owns, and the iterator
        // is refilled from scratch on every call.
        unsafe {
            crate::check(sys::ghostty_render_state_get(
                state,
                sys::GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR,
                &mut self.raw as *mut _ as *mut c_void,
            ))?;
        }

        let mut out = Vec::new();
        // SAFETY: `next` moves onto a placement or reports the end, and each
        // field is read into a local of the documented type while it is current.
        unsafe {
            while sys::ghostty_render_state_image_iterator_next(self.raw) {
                let Some(placement) = read_placement(self.raw) else {
                    continue;
                };
                out.push(placement);
            }
        }

        out.sort_by_key(|p| (layer_rank(p.layer), p.z));
        Ok(out)
    }
}

impl Drop for ImageIterator {
    fn drop(&mut self) {
        // SAFETY: the handle came from `_new` and is freed exactly once.
        unsafe { sys::ghostty_render_state_image_iterator_free(self.raw) };
    }
}

/// Draw order within the result: the layer's own value is the draw order, so
/// sorting by it puts the bottom-most layer first.
fn layer_rank(layer: ImageLayer) -> u8 {
    match layer {
        ImageLayer::BelowBackground => 0,
        ImageLayer::BelowText => 1,
        ImageLayer::AboveText => 2,
    }
}

/// The library's layer constant as this crate's enum.
fn to_layer(layer: i32) -> ImageLayer {
    match layer {
        sys::GHOSTTY_IMAGE_LAYER_BELOW_BACKGROUND => ImageLayer::BelowBackground,
        sys::GHOSTTY_IMAGE_LAYER_ABOVE_TEXT => ImageLayer::AboveText,
        // Below-text is the documented middle, and it is also the safe default:
        // an unknown layer drawn under the text cannot hide what the terminal
        // is saying.
        _ => ImageLayer::BelowText,
    }
}

/// Reads one placement, or `None` if the library reports a value this version
/// does not know how to interpret.
///
/// # Safety
/// The iterator must be sitting on a placement.
unsafe fn read_placement(iterator: sys::GhosttyRenderStateImageIterator) -> Option<ImagePlacement> {
    let mut image_id = 0u32;
    let mut generation = 0u64;
    let mut layer = 0i32;
    let mut z = 0i32;
    let mut col = 0i32;
    let mut row = 0i32;
    let mut cols = 0u32;
    let mut rows = 0u32;
    let mut offset_x = 0u32;
    let mut offset_y = 0u32;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut source_x = 0u32;
    let mut source_y = 0u32;
    let mut source_width = 0u32;
    let mut source_height = 0u32;

    // A field the library does not have leaves its local at zero, which is the
    // right answer for every one of them: no image, the first layer, the whole
    // source rectangle.
    //
    // Edition 2024 does not make an `unsafe fn` body an unsafe context, so the
    // reads are wrapped rather than relying on the signature.
    macro_rules! read {
        ($key:expr, $out:expr) => {
            unsafe {
                sys::ghostty_render_state_image_get(iterator, $key, $out as *mut _ as *mut c_void)
            }
        };
    }

    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_ID, &mut image_id);
    read!(
        sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_GENERATION,
        &mut generation
    );
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_LAYER, &mut layer);
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_Z, &mut z);
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_COL, &mut col);
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_ROW, &mut row);
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_GRID_COLS, &mut cols);
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_GRID_ROWS, &mut rows);
    read!(
        sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_X,
        &mut offset_x
    );
    read!(
        sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_Y,
        &mut offset_y
    );
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_WIDTH, &mut width);
    read!(
        sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_HEIGHT,
        &mut height
    );
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_X, &mut source_x);
    read!(sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_Y, &mut source_y);
    read!(
        sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_WIDTH,
        &mut source_width
    );
    read!(
        sys::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_HEIGHT,
        &mut source_height
    );

    let layer = to_layer(layer);

    // A source rectangle of zero means "the whole image", which is what the
    // library reports for the common case; filling it in here keeps a renderer
    // from having to know that.
    let source_width = if source_width == 0 {
        width
    } else {
        source_width
    };
    let source_height = if source_height == 0 {
        height
    } else {
        source_height
    };

    Some(ImagePlacement {
        image_id,
        generation,
        layer,
        z,
        col,
        row,
        cols,
        rows,
        offset_x,
        offset_y,
        width,
        height,
        source_x,
        source_y,
        source_width,
        source_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every format the library can report has to widen to four channels, or
    /// the texture upload would read past the end of the pixel buffer.
    #[test]
    fn every_format_reports_a_byte_count() {
        assert_eq!(ImageFormat::Rgb.bytes_per_pixel(), 3);
        assert_eq!(ImageFormat::Rgba.bytes_per_pixel(), 4);
        assert_eq!(ImageFormat::GrayAlpha.bytes_per_pixel(), 2);
        assert_eq!(ImageFormat::Gray.bytes_per_pixel(), 1);
    }

    /// The library never reports PNG, because it decodes payloads before
    /// storing them. Reading it back must not silently become another format.
    #[test]
    fn png_is_not_a_stored_format() {
        assert_eq!(ImageFormat::from_sys(2), None);
    }

    /// A format this version does not know is `None` rather than a guess: the
    /// caller drops the placement instead of drawing it with the wrong layout.
    #[test]
    fn an_unknown_format_is_none() {
        assert_eq!(ImageFormat::from_sys(99), None);
        assert_eq!(ImageFormat::from_sys(0), Some(ImageFormat::Rgb));
        assert_eq!(ImageFormat::from_sys(4), Some(ImageFormat::Gray));
    }

    /// A source rectangle of zero means "the whole image". Filling it in here
    /// keeps every renderer from having to know that rule.
    #[test]
    fn an_empty_source_rectangle_means_the_whole_image() {
        assert_eq!(fallback(0, 640), 640);
        assert_eq!(fallback(320, 640), 320);
    }

    fn fallback(reported: u32, drawn: u32) -> u32 {
        if reported == 0 { drawn } else { reported }
    }

    /// The three layers are drawn bottom first, and the sort has to agree with
    /// that: an image that ends up in the wrong layer still looks right until
    /// something covers it.
    #[test]
    fn layers_sort_bottom_first() {
        assert!(layer_rank(ImageLayer::BelowBackground) < layer_rank(ImageLayer::BelowText));
        assert!(layer_rank(ImageLayer::BelowText) < layer_rank(ImageLayer::AboveText));
    }

    /// The library's layer constants are 0, 1, 2 in draw order. Getting them
    /// wrong silently inverts which images go under the text.
    #[test]
    fn every_layer_round_trips_through_its_constant() {
        let cases = [
            (
                sys::GHOSTTY_IMAGE_LAYER_BELOW_BACKGROUND,
                ImageLayer::BelowBackground,
            ),
            (sys::GHOSTTY_IMAGE_LAYER_BELOW_TEXT, ImageLayer::BelowText),
            (sys::GHOSTTY_IMAGE_LAYER_ABOVE_TEXT, ImageLayer::AboveText),
        ];
        for (raw, expected) in cases {
            assert_eq!(to_layer(raw), expected);
        }
        assert_eq!(
            (
                sys::GHOSTTY_IMAGE_LAYER_BELOW_BACKGROUND,
                sys::GHOSTTY_IMAGE_LAYER_BELOW_TEXT,
                sys::GHOSTTY_IMAGE_LAYER_ABOVE_TEXT,
            ),
            (0, 1, 2)
        );
    }

    /// An unknown layer is drawn under the text: a placement drawn too low
    /// cannot hide what the terminal is saying.
    #[test]
    fn an_unknown_layer_falls_back_to_below_text() {
        assert_eq!(to_layer(99), ImageLayer::BelowText);
    }
}
