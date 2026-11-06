//! The VT side of a session: a thread that owns `libghostty-vt`'s `Terminal`
//! and `RenderState`, parses the PTY's bytes and publishes a plain-data frame
//! the UI thread can draw.
//!
//! Everything in this module is `Send` except the terminal itself, which never
//! leaves its thread (`libghostty-vt` is `!Send` / `!Sync`).

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libghostty_vt::key;
use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::style::Underline;
use libghostty_vt::{RenderState, Terminal};

/// One cell of a frame, flattened for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct CellData {
    pub text: String,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub has_bg: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub selected: bool,
}

impl CellData {
    fn blank(fg: [u8; 3], bg: [u8; 3]) -> Self {
        Self {
            text: String::new(),
            fg,
            bg,
            has_bg: false,
            bold: false,
            italic: false,
            underline: false,
            selected: false,
        }
    }
}

/// A complete viewport, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<CellData>,
    pub cursor: Option<(u16, u16)>,
    pub cursor_visible: bool,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
}

/// Work the UI sends to the VT thread.
pub enum Cmd {
    /// A key, encoded by libghostty so terminal modes (application cursor
    /// keys, kitty keyboard protocol) are respected.
    Key { key: key::Key, mods: key::Mods },
    /// A single character typed on the keyboard, encoded as UTF-8.
    Text(String),
    /// The viewport changed size.
    Resize {
        cols: u16,
        rows: u16,
        cell_w: u32,
        cell_h: u32,
    },
    /// Extend the selection to a viewport cell (`col`, `row`).
    Select {
        anchor: (u16, u32),
        focus: (u16, u32),
    },
    /// Select the whole buffer.
    SelectAll,
    /// Drop the selection.
    ClearSelection,
    /// Scroll the viewport by `delta` rows (negative: into history).
    Scroll(i32),
}

/// What the UI reads between frames.
#[derive(Default)]
pub struct Shared {
    pub latest: Mutex<Option<Arc<Frame>>>,
    pub title: Mutex<Option<String>>,
    pub frame_ms: Mutex<f32>,
    /// The selected text as of the last change.
    pub selection: Mutex<String>,
}

/// Starts the VT thread. Returns the command sender and the shared outputs.
pub fn spawn(
    cols: u16,
    rows: u16,
    cell_w: u32,
    cell_h: u32,
    pty_rx: Receiver<Vec<u8>>,
    pty_tx: Sender<Vec<u8>>,
) -> (Sender<Cmd>, Arc<Shared>) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let shared = Arc::new(Shared::default());

    let shared_thread = shared.clone();
    std::thread::Builder::new()
        .name("myterm-vt".into())
        .spawn(move || {
            run(
                cols,
                rows,
                cell_w,
                cell_h,
                pty_rx,
                pty_tx,
                cmd_rx,
                shared_thread,
            );
        })
        .expect("spawn VT thread");

    (cmd_tx, shared)
}

#[allow(clippy::too_many_arguments)]
fn run(
    cols: u16,
    rows: u16,
    cell_w: u32,
    cell_h: u32,
    pty_rx: Receiver<Vec<u8>>,
    pty_tx: Sender<Vec<u8>>,
    cmd_rx: Receiver<Cmd>,
    shared: Arc<Shared>,
) {
    let mut term = match Terminal::new(cols, rows) {
        Ok(t) => t,
        Err(e) => {
            log::error!("terminal: {e}");
            return;
        }
    };
    // Pixel size matters for image protocols and size reports (CSI 14 t).
    let _ = term.resize(cols, rows, cell_w, cell_h);

    // Query replies (DA1, DSR, OSC 52…) go back to the shell.
    let reply_tx = pty_tx.clone();
    if let Err(e) = term.on_pty_write(move |_term: &Terminal<'_, '_>, data: &[u8]| {
        let _ = reply_tx.send(data.to_vec());
    }) {
        log::error!("on_pty_write: {e}");
    }

    // The terminal's stderr-like diagnostics.
    if let Err(e) = term.on_bell({
        let shared = shared.clone();
        move |_term: &Terminal<'_, '_>| {
            let _ = &shared;
        }
    }) {
        log::error!("on_bell: {e}");
    }

    let mut state = match RenderState::new() {
        Ok(s) => s,
        Err(e) => {
            log::error!("render state: {e}");
            return;
        }
    };
    let mut row_it = match RowIterator::new() {
        Ok(r) => r,
        Err(e) => {
            log::error!("row iterator: {e}");
            return;
        }
    };
    let mut cell_it = match CellIterator::new() {
        Ok(c) => c,
        Err(e) => {
            log::error!("cell iterator: {e}");
            return;
        }
    };
    let mut encoder = match key::Encoder::new() {
        Ok(e) => e,
        Err(e) => {
            log::error!("key encoder: {e}");
            return;
        }
    };
    let mut event = match key::Event::new() {
        Ok(e) => e,
        Err(e) => {
            log::error!("key event: {e}");
            return;
        }
    };

    let mut out: Vec<u8> = Vec::with_capacity(64);
    let mut first = true;
    let mut last_capture = Instant::now() - Duration::from_millis(100);
    let mut last_beat = Instant::now();
    let mut bytes_total = 0_u64;
    let mut frames_total = 0_u64;
    let mut first_frame = true;

    loop {
        let mut dirty = first;
        first = false;

        // PTY bytes: drain everything available, parse in one batch.
        loop {
            match pty_rx.try_recv() {
                Ok(chunk) => {
                    bytes_total += chunk.len() as u64;
                    let t = Instant::now();
                    term.vt_write(&chunk);
                    let ms = t.elapsed().as_secs_f32() * 1000.0;
                    if ms > 25.0 {
                        log::debug!("vt_write {} bytes took {ms:.1} ms", chunk.len());
                    }
                    dirty = true;
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }

        // Commands from the UI.
        loop {
            match cmd_rx.try_recv() {
                Ok(Cmd::Key { key, mods }) => {
                    event.set_action(key::Action::Press);
                    event.set_key(key);
                    event.set_mods(mods);
                    event.set_utf8::<String>(None);
                    encoder.set_options_from_terminal(&term);
                    out.clear();
                    if encoder.encode_to_vec(&event, &mut out).is_ok() && !out.is_empty() {
                        let _ = pty_tx.send(out.clone());
                    }
                }
                Ok(Cmd::Text(text)) => {
                    // One character goes through the encoder (it knows about
                    // the terminal's modes); longer text is written as UTF-8.
                    let mut chars = text.chars();
                    let single = match (chars.next(), chars.next()) {
                        (Some(c), None) => Some(c),
                        _ => None,
                    };
                    if let Some(c) = single {
                        event.set_action(key::Action::Press);
                        event.set_key(key::Key::Unidentified);
                        event.set_mods(key::Mods::empty());
                        event.set_utf8(Some(c.to_string()));
                        encoder.set_options_from_terminal(&term);
                        out.clear();
                        if encoder.encode_to_vec(&event, &mut out).is_ok() && !out.is_empty() {
                            let _ = pty_tx.send(out.clone());
                        }
                    } else if !text.is_empty() {
                        let _ = pty_tx.send(text.into_bytes());
                    }
                }
                Ok(Cmd::Resize {
                    cols,
                    rows,
                    cell_w,
                    cell_h,
                }) => {
                    if let Err(e) = term.resize(cols, rows, cell_w, cell_h) {
                        log::warn!("resize: {e}");
                    }
                    dirty = true;
                }
                Ok(Cmd::Select { anchor, focus }) => {
                    match select(&term, anchor, focus) {
                        Ok(text) => {
                            if let Ok(mut slot) = shared.selection.lock() {
                                *slot = text;
                            }
                        }
                        Err(e) => log::warn!("select: {e}"),
                    }
                    dirty = true;
                }
                Ok(Cmd::SelectAll) => {
                    match select_all(&term) {
                        Ok(text) => {
                            log::info!("selected {} chars", text.chars().count());
                            if let Ok(mut slot) = shared.selection.lock() {
                                *slot = text;
                            }
                        }
                        Err(e) => log::warn!("select all: {e}"),
                    }
                    dirty = true;
                }
                Ok(Cmd::ClearSelection) => {
                    let _ = term.set_selection(None);
                    if let Ok(mut slot) = shared.selection.lock() {
                        slot.clear();
                    }
                    dirty = true;
                }
                Ok(Cmd::Scroll(delta)) => {
                    term.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Delta(
                        delta as isize,
                    ));
                    dirty = true;
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }

        // Frames go to the UI at about 60 Hz: a flood of output parses at full
        // speed while the UI only ever sees the newest screen.
        if dirty && (first_frame || last_capture.elapsed() >= Duration::from_millis(16)) {
            first_frame = false;
            last_capture = Instant::now();
            let started = Instant::now();
            let cap = capture(&term, &mut state, &mut row_it, &mut cell_it);
            let cap_ms = started.elapsed().as_secs_f32() * 1000.0;
            if cap_ms > 25.0 {
                log::debug!("capture took {cap_ms:.1} ms");
            }
            match cap {
                Ok(frame) => {
                    frames_total += 1;
                    if let Ok(mut slot) = shared.latest.lock() {
                        *slot = Some(Arc::new(frame));
                    }
                    if let Ok(mut ms) = shared.frame_ms.lock() {
                        *ms = started.elapsed().as_secs_f32() * 1000.0;
                    }
                }
                Err(e) => log::warn!("capture: {e}"),
            }
            if let Ok(title) = term.title() {
                if let Ok(text) = std::str::from_utf8(title) {
                    if let Ok(mut slot) = shared.title.lock() {
                        let owned = text.to_owned();
                        if slot.as_deref() != Some(owned.as_str()) {
                            *slot = Some(owned);
                        }
                    }
                }
            }
        }

        if last_beat.elapsed() >= Duration::from_secs(2) {
            last_beat = Instant::now();
            log::debug!("vt: {bytes_total} bytes, {frames_total} frames");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn capture<'a>(
    term: &Terminal<'a, 'a>,
    state: &mut RenderState<'a>,
    row_it: &mut RowIterator<'a>,
    cell_it: &mut CellIterator<'a>,
) -> libghostty_vt::error::Result<Frame> {
    let snapshot = state.update(term)?;
    let colors = snapshot.colors()?;
    let cols = snapshot.cols()?;
    let rows = snapshot.rows()?;
    let background = [colors.background.r, colors.background.g, colors.background.b];
    let foreground = [colors.foreground.r, colors.foreground.g, colors.foreground.b];

    let mut cells = Vec::with_capacity(usize::from(cols) * usize::from(rows));
    let mut text = String::with_capacity(16);
    let mut row_iteration = row_it.update(&snapshot)?;

    while let Some(row) = row_iteration.next() {
        let selection = row.selection()?;
        let mut column = 0_u16;
        let mut cell_iteration = cell_it.update(row)?;
        while let Some(cell) = cell_iteration.next() {
            let selected = selection
                .as_ref()
                .is_some_and(|s| column >= s.start_x && column <= s.end_x);
            column += 1;
            if cell.graphemes_len()? == 0 {
                let mut blank = CellData::blank(foreground, background);
                blank.selected = selected;
                cells.push(blank);
                continue;
            }
            cell.graphemes_utf8(&mut text)?;

            let mut fg = cell.fg_color()?.unwrap_or(colors.foreground);
            let mut bg = cell.bg_color()?.unwrap_or(colors.background);
            let has_bg = cell.bg_color()?.is_some();
            let mut has_bg = has_bg;
            let mut bold = false;
            let mut italic = false;
            let mut underline = false;

            if cell.has_styling()? {
                let style = cell.style()?;
                bold = style.bold;
                italic = style.italic;
                underline = style.underline != Underline::None;
                if style.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                    has_bg = true;
                }
                if style.invisible {
                    continue;
                }
            }

            cells.push(CellData {
                text: text.clone(),
                fg: [fg.r, fg.g, fg.b],
                bg: [bg.r, bg.g, bg.b],
                has_bg,
                bold,
                italic,
                underline,
                selected,
            });
        }
    }

    let cursor = snapshot.cursor()?;
    Ok(Frame {
        cols,
        rows,
        cells,
        cursor: cursor.viewport.map(|v| (v.x, v.y)),
        cursor_visible: cursor.visible,
        background,
        foreground,
    })
}

/// Installs a selection between two viewport cells and returns its text.
fn select(
    term: &Terminal<'_, '_>,
    anchor: (u16, u32),
    focus: (u16, u32),
) -> libghostty_vt::error::Result<String> {
    use libghostty_vt::selection::Selection;
    use libghostty_vt::terminal::{Point, PointCoordinate};

    let start = term.grid_ref(Point::Viewport(PointCoordinate {
        x: anchor.0,
        y: anchor.1,
    }))?;
    let end = term.grid_ref(Point::Viewport(PointCoordinate {
        x: focus.0,
        y: focus.1,
    }))?;
    let selection = Selection::new(start, end, false);
    term.set_selection(Some(&selection))?;
    selection_text(term)
}

fn select_all(term: &Terminal<'_, '_>) -> libghostty_vt::error::Result<String> {
    let selection = term.select_all()?;
    term.set_selection(selection.as_ref())?;
    selection_text(term)
}

fn selection_text(term: &Terminal<'_, '_>) -> libghostty_vt::error::Result<String> {
    use libghostty_vt::fmt::Format;
    use libghostty_vt::selection::FormatOptions;

    let options = FormatOptions::new()
        .with_emit_format(Format::Plain)
        .with_trim(true);
    Ok(term
        .format_selection_alloc(None, options)?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default())
}
