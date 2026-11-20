//! Sixel graphics decoded into the shared image pipeline.
//!
//! The VT parser handles Kitty graphics but not sixel. This module extracts
//! complete DCS sixel sequences from PTY output and decodes them to RGBA.

use std::collections::BTreeMap;

use icy_sixel::{SixelDecoder, SixelImage};
use vt::{Image, ImageFormat, ImageLayer, ImagePlacement};

const MAX_DCS: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SixelFrame {
    pub image_id: u32,
    pub generation: u64,
    pub rows: u32,
}

pub struct SixelImages {
    decoder: SixelDecoder,
    pending: Vec<u8>,
    active: bool,
    images: BTreeMap<u32, (u64, Image)>,
    next_id: u32,
    generation: u64,
    newest: Option<SixelFrame>,
}

impl Default for SixelImages {
    fn default() -> Self {
        Self {
            decoder: SixelDecoder::new(),
            pending: Vec::new(),
            active: false,
            images: BTreeMap::new(),
            next_id: 0x8000_0000,
            generation: 1,
            newest: None,
        }
    }
}

impl SixelImages {
    pub fn clear(&mut self) {
        self.pending.clear();
        self.active = false;
        self.decoder.reset_palette();
        self.newest = None;
    }

    pub fn feed(&mut self, input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut index = 0;
        while index < input.len() {
            let byte = input[index];
            if self.active {
                if byte == 0x1b
                    && index + 1 < input.len()
                    && matches!(input[index + 1], 0x5c | 0x18 | 0x1a)
                {
                    self.pending.push(byte);
                    self.pending.push(input[index + 1]);
                    index += 2;
                    self.drain_dcs();
                    continue;
                }
                self.pending.push(byte);
                index += 1;
                if self.pending.len() >= MAX_DCS {
                    self.pending.clear();
                    self.active = false;
                    self.decoder.reset_palette();
                } else if byte == 0x9c {
                    self.drain_dcs();
                }
                continue;
            }
            if byte != 0x1b && byte != 0x90 {
                output.push(byte);
                index += 1;
                continue;
            }
            if byte == 0x1b && index + 1 < input.len() && input[index + 1] != b'P' {
                output.push(byte);
                index += 1;
                continue;
            }
            if !is_sixel(&input[index..]) {
                output.push(byte);
                index += 1;
                continue;
            }
            self.pending.clear();
            self.pending.push(byte);
            self.active = true;
            index += 1;
        }
        output
    }

    pub fn newest_frame(&self) -> Option<SixelFrame> {
        self.newest
    }

    pub fn image(&self, id: u32) -> Option<Image> {
        self.images.get(&id).map(|(_, image)| image.clone())
    }

    pub fn placement(
        &self,
        frame: SixelFrame,
        cursor_x: u16,
        cursor_y: u16,
    ) -> Option<ImagePlacement> {
        let (_, image) = self.images.get(&frame.image_id)?;
        Some(ImagePlacement {
            image_id: frame.image_id,
            generation: frame.generation,
            layer: ImageLayer::BelowText,
            z: 0,
            col: cursor_x as i32,
            row: cursor_y as i32,
            cols: 1,
            rows: frame.rows,
            offset_x: 0,
            offset_y: 0,
            width: image.width,
            height: image.height,
            source_x: 0,
            source_y: 0,
            source_width: image.width,
            source_height: image.height,
        })
    }

    fn drain_dcs(&mut self) {
        let sequence = std::mem::take(&mut self.pending);
        self.active = false;
        let Ok(image) = SixelImage::decode(&sequence) else {
            self.decoder.reset_palette();
            return;
        };
        self.store(image);
    }

    fn store(&mut self, image: SixelImage) {
        if image.width == 0
            || image.height == 0
            || image.pixels.len() < image.width * image.height * 4
        {
            return;
        }
        self.generation += 1;
        let stored = Image {
            width: image.width as u32,
            height: image.height as u32,
            format: ImageFormat::Rgba,
            pixels: image.pixels,
            generation: self.generation,
        };
        let image_id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        if self.next_id == 0 {
            self.next_id = 0x8000_0000;
        }
        self.images.insert(image_id, (stored.generation, stored));
        self.newest = Some(SixelFrame {
            image_id,
            generation: self.generation,
            rows: image.height.div_ceil(6) as u32,
        });
    }
}

fn is_sixel(input: &[u8]) -> bool {
    let mut index = if input.first() == Some(&0x90) {
        1
    } else if input.starts_with(&[0x1b, b'P']) {
        2
    } else {
        return false;
    };
    while index < input.len() {
        if input[index] == b'q' {
            return true;
        }
        if input[index].is_ascii_digit() || matches!(input[index], b';' | b':' | b'=' | b'?' | b' ')
        {
            index += 1;
            continue;
        }
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut sequence = vec![
            0x1b, b'P', b'q', b'#', b'0', b';', b'2', b';', b'1', b'0', b'0', b';', b'0', b';',
            b'0', b'#', b'0', b'~', b'-',
        ];
        sequence.extend_from_slice(&[0x1b, 0x5c]);
        sequence
    }

    #[test]
    fn extracts_a_sixel_between_text() {
        let mut images = SixelImages::default();
        let mut input = b"before".to_vec();
        input.extend_from_slice(&sample());
        input.extend_from_slice(b"after");
        assert_eq!(images.feed(&input), b"beforeafter");
        assert!(images.newest_frame().is_some());
    }

    #[test]
    fn a_sequence_can_span_chunks() {
        let mut images = SixelImages::default();
        let sequence = sample();
        let split = sequence.len() / 2;
        assert!(images.feed(&sequence[..split]).is_empty());
        assert!(images.feed(&sequence[split..]).is_empty());
        assert!(images.newest_frame().is_some());
    }

    #[test]
    fn a_non_sixel_dcs_is_left_alone() {
        let mut images = SixelImages::default();
        let sequence = b"\x1bP|X\x1b\\after";
        assert_eq!(images.feed(sequence), sequence);
        assert!(images.newest_frame().is_none());
    }

    #[test]
    fn clear_removes_the_registered_image() {
        let mut images = SixelImages::default();
        images.feed(&sample());
        let id = images.newest_frame().expect("frame").image_id;
        assert!(images.image(id).is_some());
        images.clear();
        assert!(images.newest_frame().is_none());
    }
}
