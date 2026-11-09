//! One terminal: a PTY, a VT parser and the frame cache the renderer reads.
//!
//! The VT state lives on the UI thread. Bytes from the PTY are drained on every
//! tick, fed into the parser, and the resulting rows are cached so painting
//! never has to talk to the parser.

use std::collections::HashMap;
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
        };
        session.capture();
        Ok(session)
    }

    /// Drains the PTY and refreshes the frame. Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        let (bytes, eof) = self.pty.drain();
        let mut changed = false;

        if !bytes.is_empty() {
            self.term.feed(&bytes);
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
