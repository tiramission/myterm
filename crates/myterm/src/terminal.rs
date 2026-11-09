//! One terminal: a PTY, a VT parser and the frame cache the renderer reads.
//!
//! The VT state lives on the UI thread. Bytes from the PTY are drained on every
//! tick, fed into the parser, and the resulting rows are cached so painting
//! never has to talk to the parser.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pty::{Command, Pty};
use vt::{Cursor, Image, ImagePlacement, RenderState, Rgb, Row, Scrollbar, Terminal};

/// One image the terminal has drawn, ready to paint.
///
/// The placement says where it goes; the pixels are kept beside it because they
/// belong to the terminal and are only promised until it is next modified. A
/// frame is a snapshot, so it carries its own copy — shared, so a frame that has
/// not changed does not copy the pixels again.
#[derive(Clone, Debug)]
pub struct FrameImage {
    pub placement: ImagePlacement,
    pub image: Arc<Image>,
}

/// A snapshot of what to draw.
#[derive(Clone, Debug)]
pub struct Frame {
    pub rows: Vec<Row>,
    pub cursor: Cursor,
    pub background: Rgb,
    pub foreground: Rgb,
    pub palette: [Rgb; 256],
    pub scrollbar: Scrollbar,
    pub title: String,
    /// Images to draw, in draw order: below the text first, then above it.
    pub images: Vec<FrameImage>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            cursor: Cursor::default(),
            background: Rgb::default(),
            foreground: Rgb::default(),
            palette: [Rgb::default(); 256],
            scrollbar: Scrollbar::default(),
            title: String::new(),
            images: Vec::new(),
        }
    }
}

/// A running terminal pane.
/// Keeps the terminal's cursor where the shell thinks it is.
///
/// A Kitty image is drawn in cells, and the protocol has the terminal move the
/// cursor past it — by default, and `C=1` asks for that to be skipped. The
/// console behind a pseudo-terminal on Windows implements no graphics protocol
/// at all, so it moves the cursor by the newline alone. The two then disagree by
/// the height of every image, and a shell that addresses its input line by
/// absolute row — PowerShell's does — writes its prompt into the rows the image
/// covers. What a user sees is their command echoed twice, prompts stacked up
/// the screen, and lines sliding upward as they type.
///
/// Marking every display command with `C=1` before it reaches the parser makes
/// the terminal agree with the console it cannot see: neither reserves rows, and
/// the cursor stays true. The picture still goes where the program put it and
/// still looks like itself; the text that follows simply flows over it, which is
/// what a terminal without image support would do anyway.
/// The rows the terminal is ahead of the console behind it.
///
/// The console a pseudo-terminal keeps on Windows implements no graphics
/// protocol, so an image reserves nothing there: it advances its cursor by the
/// newline alone. The terminal does reserve the rows, as the protocol says it
/// should, and stops on the image's last row. Every image therefore leaves the
/// terminal one row further down than the console for each row beyond the first,
/// and the difference stays until something resets both models — which only a
/// cleared screen does.
fn image_rows_ahead(rows: u32) -> i32 {
    (rows as i32 - 1).max(0)
}

/// Whether `bytes` clears the screen, which puts both models back at the same
/// place and so puts the count back to zero.
fn clears_screen(bytes: &[u8]) -> bool {
    // `CSI 2J` erases the display, `CSI 3J` the scrollback with it. Both are
    // followed by a move home, and after that the rows the pictures had taken
    // exist in neither model.
    bytes.windows(4).any(|w| w == b"\x1b[2J" || w == b"\x1b[3J")
}
/// Moves the absolute row addresses a program writes down by `rows`.
///
/// A program that asks the console where it is then writes an absolute row; that
/// row is the console's, and the picture has made the terminal disagree with it.
/// Adding the difference puts the text where the program meant it, which is what
/// lets a prompt sit under an image instead of on top of it.
///
/// Only `CUP` and `HVP` with explicit parameters move. `CSI H` alone means
/// "home", which is the same row in both models, and a private sequence is not a
/// cursor address at all.
fn align_rows(bytes: &[u8], rows: i32) -> Cow<'_, [u8]> {
    if rows == 0 {
        return Cow::Borrowed(bytes);
    }

    let mut out = Vec::with_capacity(bytes.len() + 8);
    let mut moved = false;
    let mut i = 0usize;

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit() || bytes[j] == b';' || bytes[j] == b'?')
            {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'H' || bytes[j] == b'f') {
                let params = &bytes[i + 2..j];
                let first = params
                    .iter()
                    .position(|b| *b == b';')
                    .unwrap_or(params.len());
                if !params[..first].is_empty()
                    && !params.contains(&b'?')
                    && let Ok(row) = std::str::from_utf8(&params[..first])
                        .unwrap_or("")
                        .parse::<i32>()
                {
                    out.extend_from_slice(format!("\x1b[{}", (row + rows).max(1)).as_bytes());
                    out.extend_from_slice(&params[first..]);
                    out.push(bytes[j]);
                    moved = true;
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    if moved {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(bytes)
    }
}

pub struct Session {
    pty: Pty,
    term: Terminal,
    state: RenderState,
    frame: Frame,
    /// Columns and rows the PTY currently has.
    size: (u16, u16),
    /// Set once the child process exits.
    exit: Option<i32>,
    /// Bytes the parser wants to send back to the program. Shared with the
    /// write-pty closure, which the parser calls while `feed` is running.
    pending_write: Arc<Mutex<Vec<u8>>>,
    /// When the program last wrote anything. What the sidebar shows as
    /// "working" is simply output that arrived a moment ago.
    last_output: Instant,
    /// Pixels of every image the terminal currently holds, keyed by image id and
    /// stamped with the generation they were read at.
    ///
    /// The generation is what keeps this cheap. A program that sends an image
    /// and then keeps typing does not change it, so the pixels are read once and
    /// shared by every frame after that.
    image_pixels: HashMap<u32, (u64, Arc<Image>)>,
    /// How many rows the images have moved this terminal past the console
    /// behind it, accumulated since the screen was last cleared.
    rows_apart: i32,
    /// The images already counted into `rows_apart`, by id and generation, so a
    /// frame that still shows them does not count them twice.
    counted_images: HashSet<(u32, u64)>,
}

impl Session {
    /// Starts `command` and returns a session sized to `cols`×`rows`.
    pub fn spawn(command: &Command, cols: u16, rows: u16) -> Result<Self, String> {
        let pty = Pty::spawn(command, cols.max(2), rows.max(2))?;
        Self::from_pty(pty, cols, rows)
    }

    /// Wraps an already-running pseudo console.
    ///
    /// Split out from [`Session::spawn`] because starting the console is by far
    /// the slowest step — around 45 ms of the 50 ms total — while everything
    /// here costs a fraction of a millisecond. That lets the caller open the
    /// console on a background thread and finish the session on the UI thread,
    /// so no frame is blocked for the duration.
    pub fn from_pty(pty: Pty, cols: u16, rows: u16) -> Result<Self, String> {
        let cols = cols.max(2);
        let rows = rows.max(2);

        // The parser answers device queries by writing to the PTY. The closure
        // runs inside `feed`, while the parser still borrows the terminal, so it
        // appends to a shared buffer that `poll` flushes afterwards.
        let pending_write = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = Arc::clone(&pending_write);
        let term = Terminal::new(
            cols,
            rows,
            Some(Box::new(move |bytes: &[u8]| {
                if let Ok(mut buf) = sink.lock() {
                    buf.extend_from_slice(bytes);
                }
            })),
        )
        .map_err(|e| format!("terminal: {e}"))?;

        let state = RenderState::new().map_err(|e| format!("render state: {e}"))?;

        let mut session = Self {
            pty,
            term,
            state,
            frame: Frame::default(),
            size: (cols, rows),
            exit: None,
            pending_write,
            last_output: Instant::now(),
            image_pixels: HashMap::new(),
            rows_apart: 0,
            counted_images: HashSet::new(),
        };
        session.capture();
        Ok(session)
    }

    /// Drains the PTY and refreshes the frame. Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        let (bytes, eof) = self.pty.drain();
        let mut changed = false;

        if !bytes.is_empty() {
            // A cleared screen puts both models back at the top, so the rows the
            // pictures had taken stop counting — before the bytes that do the
            // clearing are mapped, or the move home that follows them would be
            // pushed down by rows that no longer exist.
            if clears_screen(&bytes) {
                self.rows_apart = 0;
                // The pictures may outlive an erase; they must not be counted a
                // second time when they show up again.
                for image in &self.frame.images {
                    self.counted_images
                        .insert((image.placement.image_id, image.placement.generation));
                }
            }

            // The frame from the last paint says how far the picture has moved
            // the terminal past the console; the bytes arriving now may write
            // absolute rows that were measured against that console.
            let rows = self.rows_apart;
            self.term.feed(&align_rows(&bytes, rows));
            self.last_output = Instant::now();
            changed = true;
        }

        if let Ok(mut pending) = self.pending_write.lock()
            && !pending.is_empty()
        {
            self.pty.write(&pending);
            pending.clear();
        }

        if eof && self.exit.is_none() {
            self.exit = self.pty.exit_code().or(Some(-1));
            changed = true;
        }

        if changed {
            self.capture();
        }

        changed
    }

    /// Re-reads the parser into the frame cache.
    fn capture(&mut self) {
        if self.state.update(self.term.raw()).is_err() {
            return;
        }

        let colors = self.state.colors();
        let cursor = self.state.cursor();

        let mut rows = Vec::new();
        self.state.for_each_row(|row| rows.push(row));

        let images = self.collect_images();

        self.term.refresh_title();
        self.frame = Frame {
            rows,
            cursor,
            background: colors.background,
            foreground: colors.foreground,
            palette: colors.palette,
            scrollbar: self.term.scrollbar(),
            title: self.term.title().to_owned(),
            images,
        };
        // New pictures push the terminal further down than the console; ones
        // already counted keep their place in that count.
        for image in &self.frame.images {
            let key = (image.placement.image_id, image.placement.generation);
            if self.counted_images.insert(key) {
                self.rows_apart += image_rows_ahead(image.placement.rows);
            }
        }
    }

    /// Resolves every placement the render state reports into pixels.
    ///
    /// The pixels are read from the terminal, which only promises they stay put
    /// until it is next modified — so they are copied out here, once, and then
    /// shared by every frame until the generation moves.
    fn collect_images(&mut self) -> Vec<FrameImage> {
        let placements = self.state.images();
        let mut images = Vec::with_capacity(placements.len());

        for placement in placements {
            let cached = self
                .image_pixels
                .get(&placement.image_id)
                .filter(|(generation, _)| *generation == placement.generation);
            let image = match cached {
                Some((_, image)) => Arc::clone(image),
                None => {
                    let Some(image) = self.term.kitty_image(placement.image_id) else {
                        // The render state skips placements whose pixels have not
                        // arrived, so this is a terminal that dropped the image
                        // between reporting it and now.
                        continue;
                    };
                    let image = Arc::new(image);
                    self.image_pixels.insert(
                        placement.image_id,
                        (placement.generation, Arc::clone(&image)),
                    );
                    image
                }
            };
            images.push(FrameImage { placement, image });
        }

        // Anything the terminal no longer reports has no reason to stay: an
        // animation that has moved on would otherwise accumulate every frame it
        // has ever shown.
        let live: Vec<u32> = images
            .iter()
            .map(|image| image.placement.image_id)
            .collect();
        self.image_pixels.retain(|id, _| live.contains(id));

        images
    }

    /// The current frame.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Resizes both the parser and the pseudo console.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        if (cols, rows) == self.size {
            return;
        }
        self.size = (cols, rows);
        self.term.resize(cols, rows, cell_width_px, cell_height_px);
        self.pty.resize(cols, rows);
        self.capture();
    }

    /// Writes raw bytes to the child.
    ///
    /// Pasting is typing, so it pulls the viewport back to the live end for the
    /// same reason a keystroke does.
    pub fn write(&mut self, bytes: &[u8]) {
        self.pty.write(bytes);
        self.term.scroll(vt::Scroll::Bottom);
        self.capture();
    }

    /// Encodes and writes one key press.
    ///
    /// Typing pulls the viewport back to the live end. A user who scrolled up to
    /// read something and then starts typing is typing at the bottom, and
    /// leaving the view where it was means the characters they send are not on
    /// screen — the shell echoes them into a region they cannot see.
    pub fn send_key(&mut self, input: vt::KeyInput<'_>) {
        let bytes = self.term.encode_key(input);
        if bytes.is_empty() {
            return;
        }
        self.pty.write(&bytes);
        self.term.scroll(vt::Scroll::Bottom);
        self.capture();
    }

    /// Scrolls the viewport by `delta` rows (negative is older).
    pub fn scroll(&mut self, delta: isize) {
        self.term.scroll(vt::Scroll::Delta(delta));
        self.capture();
    }

    /// Sets the selection from two viewport cells and refreshes the frame.
    pub fn set_selection(&mut self, start: (u16, u32), end: (u16, u32)) {
        self.term.set_selection(start, end);
        self.capture();
    }

    /// Drops the selection and refreshes the frame.
    pub fn clear_selection(&mut self) {
        self.term.clear_selection();
        self.capture();
    }

    /// The selected text, if anything is selected.
    pub fn selected_text(&mut self) -> Option<String> {
        self.term.selected_text()
    }

    /// How long ago the program last wrote.
    pub fn idle_for(&self) -> Duration {
        self.last_output.elapsed()
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit
    }

    /// Applies the colors the app wants for this terminal.
    pub fn set_colors(&mut self, fg: Rgb, bg: Rgb, cursor: Option<Rgb>, palette: &[Rgb; 256]) {
        self.term.set_colors(fg, bg, cursor, palette);
        self.capture();
    }
}

#[cfg(test)]
mod align_tests {
    use super::{align_rows, clears_screen, image_rows_ahead};

    /// An image of `rows` moves the terminal one row further than the console
    /// for every row beyond the first. One row is no distance at all.
    #[test]
    fn an_image_counts_all_but_its_first_row() {
        assert_eq!(image_rows_ahead(0), 0);
        assert_eq!(image_rows_ahead(1), 0);
        assert_eq!(image_rows_ahead(7), 6);
        assert_eq!(image_rows_ahead(20), 19);
    }

    /// Only an erase puts the two models back together.
    #[test]
    fn only_an_erase_resets_the_count() {
        assert!(clears_screen(b"[2J[1;1H"));
        assert!(clears_screen(b"[3J"));
        assert!(!clears_screen(b"[H"));
        assert!(!clears_screen(b"[1;1H"));
        assert!(!clears_screen(b"plain text"));
    }

    /// A row address a program wrote down moves with the picture.
    #[test]
    fn an_absolute_row_is_moved() {
        assert_eq!(align_rows(b"\x1b[4;26H", 6).as_ref(), b"\x1b[10;26H");
        assert_eq!(align_rows(b"\x1b[1;1f", 3).as_ref(), b"\x1b[4;1f");
    }

    /// Text and other sequences are left exactly as they are.
    #[test]
    fn everything_else_passes_through() {
        for input in [
            &b"plain text\r\n"[..],
            &b"\x1b[1m bold \x1b[0m"[..],
            &b"\x1b[?25l"[..],
            // Home is the same row in both models, and a private sequence is
            // not a cursor address at all.
            &b"\x1b[1;1H"[..],
            &b"\x1b[?62;22c"[..],
        ] {
            // `CSI 1;1H` is a real address and does move; the rest must not.
            let out = align_rows(input, 4);
            if input == b"\x1b[1;1H" {
                assert_eq!(out.as_ref(), b"\x1b[5;1H");
            } else {
                assert_eq!(out.as_ref(), input, "changed: {input:?}");
            }
        }
    }

    /// With nothing to correct, the bytes are handed on untouched rather than
    /// copied — this runs on every read.
    #[test]
    fn nothing_to_do_is_borrowed() {
        assert!(matches!(
            align_rows(b"\x1b[4;26H", 0),
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(matches!(
            align_rows(b"plain text", 5),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    /// Several addresses in one read all move, and the text between them is
    /// kept in place.
    #[test]
    fn every_address_in_one_read_moves() {
        assert_eq!(
            align_rows(b"\x1b[2;1Habc\x1b[5;2H", 3).as_ref(),
            b"\x1b[5;1Habc\x1b[8;2H"
        );
    }
}
