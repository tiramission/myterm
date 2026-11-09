//! Safe ownership wrappers over libghostty-vt.
//!
//! This crate is the only place that knows about raw pointers. Everything it
//! exposes is a value type or an owner that frees on `Drop`, so the app never
//! has to reason about the C lifetime rules.

use std::ffi::c_void;

use vt_sys as sys;

/// An 8-bit RGB color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_array(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }

    fn from_sys(c: sys::GhosttyColorRgb) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }

    fn to_sys(self) -> sys::GhosttyColorRgb {
        sys::GhosttyColorRgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

/// A failure reported by the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    OutOfMemory,
    InvalidValue,
    OutOfSpace,
    NoValue,
    Io,
    LimitExceeded,
    Rejected,
    Unsupported,
    Unknown(i32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::OutOfMemory => "out of memory",
            Self::InvalidValue => "invalid value",
            Self::OutOfSpace => "out of space",
            Self::NoValue => "no value",
            Self::Io => "io error",
            Self::LimitExceeded => "limit exceeded",
            Self::Rejected => "rejected",
            Self::Unsupported => "unsupported",
            Self::Unknown(code) => return write!(f, "unknown error {code}"),
        };
        f.write_str(text)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn check(code: sys::GhosttyResult) -> Result<()> {
    match code {
        sys::GHOSTTY_SUCCESS => Ok(()),
        sys::GHOSTTY_OUT_OF_MEMORY => Err(Error::OutOfMemory),
        sys::GHOSTTY_INVALID_VALUE => Err(Error::InvalidValue),
        sys::GHOSTTY_OUT_OF_SPACE => Err(Error::OutOfSpace),
        sys::GHOSTTY_NO_VALUE => Err(Error::NoValue),
        sys::GHOSTTY_IO_ERROR => Err(Error::Io),
        sys::GHOSTTY_LIMIT_EXCEEDED => Err(Error::LimitExceeded),
        sys::GHOSTTY_REJECTED => Err(Error::Rejected),
        sys::GHOSTTY_UNSUPPORTED => Err(Error::Unsupported),
        other => Err(Error::Unknown(other)),
    }
}

/// How the terminal should move its viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    Top,
    Bottom,
    Delta(isize),
}

/// The cursor's shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    Bar,
    #[default]
    Block,
    Underline,
    BlockHollow,
}

impl CursorStyle {
    fn from_sys(style: i32) -> Self {
        match style {
            sys::GHOSTTY_CURSOR_STYLE_BAR => Self::Bar,
            sys::GHOSTTY_CURSOR_STYLE_UNDERLINE => Self::Underline,
            sys::GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW => Self::BlockHollow,
            _ => Self::Block,
        }
    }
}

/// Cursor placement and appearance for one render pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub in_viewport: bool,
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub blinking: bool,
    pub wide_tail: bool,
    pub style: CursorStyle,
}

/// The render state's resolved default colors.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Option<Rgb>,
    pub palette: [Rgb; 256],
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: Rgb::default(),
            foreground: Rgb::default(),
            cursor: None,
            palette: [Rgb::default(); 256],
        }
    }
}

/// One cell's resolved appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: bool,
}

/// One cell's contents and appearance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub style: CellStyle,
    /// True when the cell is covered by the active selection.
    pub selected: bool,
    /// True for the second half of a double-width character.
    pub wide_tail: bool,
}

/// Scrollbar geometry, in rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// One row of cells.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The row's position from the top of the viewport.
    pub viewport_y: i32,
}

/// A parsed key press, ready to encode.
#[derive(Clone, Copy, Debug)]
pub struct KeyInput<'a> {
    pub key: i32,
    pub mods: u16,
    pub utf8: Option<&'a str>,
}

/// A terminal, owning the parser and scrollback.
///
/// The `write_pty` callback is invoked when the terminal wants to answer the
/// host program (device status reports, mode queries). It receives raw bytes.
/// The callback that carries terminal output to the PTY.
///
/// Named because it is boxed twice and written out four times, and the spelling
/// is where a mistake would hide.
type WritePty = Box<dyn FnMut(&[u8]) + 'static>;

pub struct Terminal {
    raw: sys::GhosttyTerminal,
    cols: u16,
    rows: u16,
    /// Boxed so the pointer handed to the library stays valid; the library
    /// stores the address, so this must not move after installation.
    hooks: Box<Hooks>,
    encoder: KeyEncoder,
    title: String,
}

/// What the library reaches back into through `GHOSTTY_TERMINAL_OPT_USERDATA`.
///
/// One struct rather than two pointers, because the library has a single
/// userdata slot and more than one callback wants it.
struct Hooks {
    /// Where the parser's answers go, so a program can hear them. `None` in
    /// tests and anywhere the terminal has no program behind it.
    write: Option<WritePty>,
    /// What to report when a program asks how big the terminal is.
    ///
    /// This matters more than it looks: a graphics program sizes its cells from
    /// the cell size it is told, and if it is told nothing it guesses — which is
    /// how an image ends up reserving the wrong number of rows.
    size: sys::GhosttySizeReportSize,
}

impl Terminal {
    /// Creates a terminal that reports its answers through `write_pty`.
    pub fn new(cols: u16, rows: u16, write_pty: Option<WritePty>) -> Result<Self> {
        let cols = cols.max(1);
        let rows = rows.max(1);

        let mut raw: sys::GhosttyTerminal = std::ptr::null_mut();
        check(unsafe { sys::ghostty_terminal_new(std::ptr::null(), &mut raw, cols, rows) })?;

        let mut terminal = Self {
            raw,
            cols,
            rows,
            hooks: Box::new(Hooks {
                write: write_pty,
                size: sys::GhosttySizeReportSize {
                    rows,
                    columns: cols,
                    // Unknown until the owner measures the font and resizes.
                    // Reporting zero would make a program divide by it, so the
                    // callback suppresses the answer instead; see the trampoline.
                    cell_width: 0,
                    cell_height: 0,
                },
            }),
            encoder: KeyEncoder::new()?,
            title: String::new(),
        };

        terminal.install_hooks()?;

        // A blinking caret is the convention for an interactive prompt, and
        // libghostty defaults to a steady one. This only sets the default that
        // `CSI 0 q` restores: a program that asks for a specific style still
        // gets exactly what it asked for.
        let blink = true;
        unsafe {
            let _ = sys::ghostty_terminal_set(
                terminal.raw,
                sys::GHOSTTY_TERMINAL_OPT_DEFAULT_CURSOR_BLINK,
                &blink as *const _ as *const c_void,
            );
        }

        Ok(terminal)
    }

    /// Points the library at this terminal's hooks.
    ///
    /// `hooks` is a `Box`, so its address is stable for the terminal's whole
    /// life — which is what the library needs, because it keeps the pointer
    /// rather than a copy.
    fn install_hooks(&mut self) -> Result<()> {
        let userdata = &mut *self.hooks as *mut Hooks as *mut c_void;

        unsafe {
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_USERDATA,
                userdata,
            ))?;
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                write_pty_trampoline as *const c_void,
            ))?;
            // Answers `CSI 14/16/18/19 t`. A graphics program asks these to
            // learn the cell size; with no answer it guesses, and guesses wrong.
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SIZE,
                size_trampoline as *const c_void,
            ))?;
        }

        Ok(())
    }

    /// Feeds bytes from the PTY into the parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        unsafe { sys::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
    }

    /// Resizes the grid and records the new cell geometry.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) {
        let cols = cols.max(1);
        let rows = rows.max(1);

        // The size the terminal reports has to follow the cell metrics too, not
        // just the cell count: a font change moves the same grid to a different
        // pixel size, and a program that asked before would keep the old number.
        self.hooks.size = sys::GhosttySizeReportSize {
            rows,
            columns: cols,
            cell_width: cell_width_px,
            cell_height: cell_height_px,
        };

        // The parser is told even when the cell count is unchanged, because the
        // pixel size may still have moved.
        unsafe {
            let _ =
                sys::ghostty_terminal_resize(self.raw, cols, rows, cell_width_px, cell_height_px);
        };

        self.cols = cols;
        self.rows = rows;
    }

    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Sets the default foreground/background/cursor colors and palette.
    pub fn set_colors(
        &mut self,
        foreground: Rgb,
        background: Rgb,
        cursor: Option<Rgb>,
        palette: &[Rgb; 256],
    ) {
        unsafe {
            let fg = foreground.to_sys();
            let _ = sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND,
                &fg as *const _ as *const c_void,
            );

            let bg = background.to_sys();
            let _ = sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND,
                &bg as *const _ as *const c_void,
            );

            match cursor {
                Some(c) => {
                    let c = c.to_sys();
                    let _ = sys::ghostty_terminal_set(
                        self.raw,
                        sys::GHOSTTY_TERMINAL_OPT_COLOR_CURSOR,
                        &c as *const _ as *const c_void,
                    );
                }
                None => {
                    let _ = sys::ghostty_terminal_set(
                        self.raw,
                        sys::GHOSTTY_TERMINAL_OPT_COLOR_CURSOR,
                        std::ptr::null(),
                    );
                }
            }

            let pal: Vec<sys::GhosttyColorRgb> = palette.iter().map(|c| c.to_sys()).collect();
            let _ = sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_COLOR_PALETTE,
                pal.as_ptr() as *const c_void,
            );
        }
    }

    /// Moves the viewport.
    pub fn scroll(&mut self, scroll: Scroll) {
        let value = match scroll {
            Scroll::Top => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_TOP,
                value: sys::GhosttyTerminalScrollViewportValue { delta: 0 },
            },
            Scroll::Bottom => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM,
                value: sys::GhosttyTerminalScrollViewportValue { delta: 0 },
            },
            Scroll::Delta(d) => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA,
                value: sys::GhosttyTerminalScrollViewportValue { delta: d },
            },
        };
        unsafe {
            let _ = sys::ghostty_terminal_scroll_viewport(self.raw, value);
        }
    }

    /// The current scrollbar geometry.
    pub fn scrollbar(&self) -> Scrollbar {
        let mut out = sys::GhosttyTerminalScrollbar::default();
        let ok = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                &mut out as *mut _ as *mut c_void,
            )
        };
        if check(ok).is_err() {
            return Scrollbar::default();
        }
        Scrollbar {
            total: out.total,
            offset: out.offset,
            len: out.len,
        }
    }

    /// Selects the whole active screen and returns its plain text.
    pub fn select_all_text(&mut self) -> Option<String> {
        let mut selection = sys::GhosttySelection {
            start: sys::GhosttyGridRef {
                node: std::ptr::null_mut(),
                x: 0,
                y: 0,
            },
            end: sys::GhosttyGridRef {
                node: std::ptr::null_mut(),
                x: 0,
                y: 0,
            },
            rectangle: false,
        };

        if check(unsafe { sys::ghostty_terminal_select_all(self.raw, &mut selection) }).is_err() {
            return None;
        }

        unsafe {
            let _ = sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                &selection as *const _ as *const c_void,
            );
        }

        let text = self.format_selection();

        unsafe {
            let _ = sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                std::ptr::null(),
            );
        }

        text
    }

    /// Replaces the selection with the given viewport-space cells.
    ///
    /// Coordinates are viewport-relative: x is a column, y a row down from the
    /// top of the visible screen. `grid_ref` turns each end into a reference the
    /// parser can hold, so the selection survives scrolling and reflow.
    pub fn set_selection(&mut self, start: (u16, u32), end: (u16, u32)) -> bool {
        let Some(start) = self.grid_ref(start.0, start.1) else {
            return false;
        };
        let Some(end) = self.grid_ref(end.0, end.1) else {
            return false;
        };

        let selection = sys::GhosttySelection {
            start,
            end,
            rectangle: false,
        };

        unsafe {
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                &selection as *const _ as *const c_void,
            ))
            .is_ok()
        }
    }

    /// Drops the selection.
    pub fn clear_selection(&mut self) {
        unsafe {
            let _ = sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                std::ptr::null(),
            );
        }
    }

    /// The current selection as plain text.
    pub fn selected_text(&mut self) -> Option<String> {
        self.format_selection()
    }

    /// Resolves a viewport cell into a grid reference.
    fn grid_ref(&self, x: u16, y: u32) -> Option<sys::GhosttyGridRef> {
        let point = sys::GhosttyPoint {
            tag: sys::GHOSTTY_POINT_TAG_VIEWPORT,
            value: sys::GhosttyPointValue {
                coordinate: sys::GhosttyPointCoordinate { x, y },
            },
        };

        let mut out = sys::GhosttyGridRef {
            node: std::ptr::null_mut(),
            x: 0,
            y: 0,
        };

        unsafe {
            check(sys::ghostty_terminal_grid_ref(self.raw, point, &mut out)).ok()?;
        }
        Some(out)
    }

    /// Formats the current selection as plain text.
    ///
    /// A formatter with no selection formats the whole screen, so the
    /// selection has to be read off the terminal and handed over explicitly.
    /// Skipping that step is why copying used to put the entire screen on the
    /// clipboard no matter which cells were highlighted.
    fn format_selection(&mut self) -> Option<String> {
        let mut selection = sys::GhosttySelection {
            start: sys::GhosttyGridRef {
                node: std::ptr::null_mut(),
                x: 0,
                y: 0,
            },
            end: sys::GhosttyGridRef {
                node: std::ptr::null_mut(),
                x: 0,
                y: 0,
            },
            rectangle: false,
        };

        let has_selection = unsafe {
            check(sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SELECTION,
                &mut selection as *mut _ as *mut c_void,
            ))
            .is_ok()
        };
        if !has_selection {
            return None;
        }

        let mut formatter: sys::GhosttyFormatter = std::ptr::null_mut();
        let ok = unsafe {
            sys::ghostty_formatter_terminal_new(std::ptr::null(), &mut formatter, self.raw)
        };
        if check(ok).is_err() {
            return None;
        }

        let result = unsafe {
            let emit = sys::GHOSTTY_FORMATTER_FORMAT_PLAIN;
            let _ = sys::ghostty_formatter_set(
                formatter,
                sys::GHOSTTY_FORMATTER_OPT_EMIT,
                &emit as *const _ as *const c_void,
            );

            let _ = sys::ghostty_formatter_set(
                formatter,
                sys::GHOSTTY_FORMATTER_OPT_SELECTION,
                &selection as *const _ as *const c_void,
            );

            // Copied text joins lines that the terminal wrapped: the wrap is a
            // property of the window width, not of the text the program wrote,
            // and pasting a paragraph broken at the old width is never useful.
            let unwrap = true;
            let _ = sys::ghostty_formatter_set(
                formatter,
                sys::GHOSTTY_FORMATTER_OPT_UNWRAP,
                &unwrap as *const _ as *const c_void,
            );

            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut len = 0usize;
            let ok = sys::ghostty_formatter_format_alloc(
                formatter,
                std::ptr::null(),
                &mut ptr,
                &mut len,
            );

            if check(ok).is_err() || ptr.is_null() {
                None
            } else {
                let bytes = std::slice::from_raw_parts(ptr, len).to_vec();
                sys::ghostty_free(std::ptr::null(), ptr, len);
                String::from_utf8(bytes).ok()
            }
        };

        unsafe { sys::ghostty_formatter_free(formatter) };
        result
    }

    /// Encodes a key press using the terminal's current modes.
    pub fn encode_key(&mut self, input: KeyInput<'_>) -> Vec<u8> {
        self.encoder.encode(self.raw, input)
    }

    pub fn raw(&self) -> sys::GhosttyTerminal {
        self.raw
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Reads one image out of the terminal's Kitty graphics storage.
    ///
    /// `None` when there is no image with that id, or when its pixels have not
    /// finished arriving. The pixels are copied out, because the library only
    /// promises they stay put until the terminal is next modified.
    pub fn kitty_image(&self, image_id: u32) -> Option<image::Image> {
        // SAFETY: a null storage yields a null image, which `from_storage`
        // checks; the handle is borrowed from this terminal and stays valid for
        // as long as the borrow does.
        unsafe {
            let mut storage: sys::GhosttyKittyGraphics = std::ptr::null_mut();
            let ok = sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS,
                &mut storage as *mut _ as *mut c_void,
            );
            if crate::check(ok).is_err() || storage.is_null() {
                return None;
            }
            image::Image::from_storage(storage, image_id)
        }
    }

    /// Re-reads the title from the terminal. The returned pointer is only valid
    /// until the next mutating call, so it is copied out here.
    pub fn refresh_title(&mut self) {
        let mut out = sys::GhosttyString {
            ptr: std::ptr::null(),
            len: 0,
        };
        let ok = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_TITLE,
                &mut out as *mut _ as *mut c_void,
            )
        };
        if check(ok).is_err() || out.ptr.is_null() || out.len == 0 {
            self.title.clear();
            return;
        }
        let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
        self.title = String::from_utf8_lossy(bytes).into_owned();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { sys::ghostty_terminal_free(self.raw) };
    }
}

extern "C" fn write_pty_trampoline(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if userdata.is_null() || data.is_null() || len == 0 {
        return;
    }
    // SAFETY: `userdata` is the `Box<Hooks>` installed in `install_hooks`, and
    // the terminal outlives the callback.
    let hooks = unsafe { &mut *(userdata as *mut Hooks) };
    let Some(cb) = hooks.write.as_mut() else {
        return;
    };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    cb(bytes);
}

/// Answers XTWINOPS size queries from the geometry last given to `resize`.
///
/// Returning false suppresses the response. That is the right answer before the
/// owner has measured the font: a cell size of zero would have a program divide
/// by it, and a wrong size is worse than no size.
extern "C" fn size_trampoline(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    out_size: *mut sys::GhosttySizeReportSize,
) -> bool {
    if userdata.is_null() || out_size.is_null() {
        return false;
    }
    // SAFETY: `userdata` is the `Box<Hooks>` installed in `install_hooks`, and
    // the out pointer is the library's own stack slot.
    let hooks = unsafe { &*(userdata as *const Hooks) };
    if hooks.size.cell_width == 0 || hooks.size.cell_height == 0 {
        return false;
    }
    unsafe { *out_size = hooks.size };
    true
}

/// A key encoder, reused across events.
pub struct KeyEncoder {
    raw: sys::GhosttyKeyEncoder,
}

impl KeyEncoder {
    pub fn new() -> Result<Self> {
        let mut raw: sys::GhosttyKeyEncoder = std::ptr::null_mut();
        check(unsafe { sys::ghostty_key_encoder_new(std::ptr::null(), &mut raw) })?;
        Ok(Self { raw })
    }

    /// Encodes one key event, syncing the encoder with the terminal's modes.
    ///
    /// The terminal handle is passed through to the C API, which is where it is
    /// dereferenced; this function only carries it. The lint assumes a raw pointer
    /// argument means a dereference here, which is not the case.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn encode(&mut self, terminal: sys::GhosttyTerminal, input: KeyInput<'_>) -> Vec<u8> {
        let mut event: sys::GhosttyKeyEvent = std::ptr::null_mut();
        if check(unsafe { sys::ghostty_key_event_new(std::ptr::null(), &mut event) }).is_err() {
            return Vec::new();
        }

        let result = unsafe {
            let _ = sys::ghostty_key_encoder_set_from_terminal(self.raw, terminal);
            let _ = sys::ghostty_key_event_set_action(event, sys::GHOSTTY_KEY_ACTION_PRESS);
            let _ = sys::ghostty_key_event_set_key(event, input.key);
            let _ = sys::ghostty_key_event_set_mods(event, input.mods);

            if let Some(text) = input.utf8 {
                let string = sys::GhosttyString {
                    ptr: text.as_ptr(),
                    len: text.len(),
                };
                let _ = sys::ghostty_key_event_set_utf8(event, string);
            }

            let mut buf = [0u8; 64];
            let mut written = 0usize;
            let ok = sys::ghostty_key_encoder_encode(
                self.raw,
                event,
                buf.as_mut_ptr(),
                buf.len(),
                &mut written,
            );

            if check(ok).is_ok() {
                buf[..written.min(buf.len())].to_vec()
            } else {
                Vec::new()
            }
        };

        unsafe { sys::ghostty_key_event_free(event) };
        result
    }
}

impl Drop for KeyEncoder {
    fn drop(&mut self) {
        unsafe { sys::ghostty_key_encoder_free(self.raw) };
    }
}

/// A render snapshot of a terminal, plus reusable iterators.
///
/// Cell data is only valid until the next [`RenderState::update`], so the
/// iterators are consumed inside the same call that refreshes them.
pub struct RenderState {
    raw: sys::GhosttyRenderState,
    rows: sys::GhosttyRenderStateRowIterator,
    cells: sys::GhosttyRenderStateRowCells,
    /// Refilled from every update and walked by `images`.
    images: image::ImageIterator,
}

impl RenderState {
    pub fn new() -> Result<Self> {
        let mut raw: sys::GhosttyRenderState = std::ptr::null_mut();
        check(unsafe { sys::ghostty_render_state_new(std::ptr::null(), &mut raw) })?;

        let mut rows: sys::GhosttyRenderStateRowIterator = std::ptr::null_mut();
        if check(unsafe { sys::ghostty_render_state_row_iterator_new(std::ptr::null(), &mut rows) })
            .is_err()
        {
            unsafe { sys::ghostty_render_state_free(raw) };
            return Err(Error::OutOfMemory);
        }

        let mut cells: sys::GhosttyRenderStateRowCells = std::ptr::null_mut();
        if check(unsafe { sys::ghostty_render_state_row_cells_new(std::ptr::null(), &mut cells) })
            .is_err()
        {
            unsafe {
                sys::ghostty_render_state_row_iterator_free(rows);
                sys::ghostty_render_state_free(raw);
            }
            return Err(Error::OutOfMemory);
        }

        let images = match image::ImageIterator::new() {
            Ok(images) => images,
            Err(error) => {
                unsafe {
                    sys::ghostty_render_state_row_cells_free(cells);
                    sys::ghostty_render_state_row_iterator_free(rows);
                    sys::ghostty_render_state_free(raw);
                }
                return Err(error);
            }
        };

        Ok(Self {
            raw,
            rows,
            cells,
            images,
        })
    }

    /// Refreshes the snapshot from the terminal.
    ///
    /// The terminal handle is only carried to the C API, not dereferenced here;
    /// see the note on `encode`.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn update(&mut self, terminal: sys::GhosttyTerminal) -> Result<()> {
        check(unsafe { sys::ghostty_render_state_update(self.raw, terminal) })
    }

    pub fn size(&self) -> (u16, u16) {
        let mut cols = 0u16;
        let mut rows = 0u16;
        unsafe {
            let _ = sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_COLS,
                &mut cols as *mut _ as *mut c_void,
            );
            let _ = sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_ROWS,
                &mut rows as *mut _ as *mut c_void,
            );
        }
        (cols, rows)
    }

    pub fn colors(&self) -> Colors {
        let mut out = sys::GhosttyRenderStateColors::default();
        let ok = unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_COLORS,
                &mut out as *mut _ as *mut c_void,
            )
        };
        if check(ok).is_err() {
            return Colors::default();
        }

        let mut palette = [Rgb::default(); 256];
        for (dst, src) in palette.iter_mut().zip(out.palette.iter()) {
            *dst = Rgb::from_sys(*src);
        }

        Colors {
            background: Rgb::from_sys(out.background),
            foreground: Rgb::from_sys(out.foreground),
            cursor: out.cursor_has_value.then(|| Rgb::from_sys(out.cursor)),
            palette,
        }
    }

    pub fn cursor(&self) -> Cursor {
        let mut out = sys::GhosttyRenderStateCursor::default();
        let ok = unsafe {
            sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_CURSOR,
                &mut out as *mut _ as *mut c_void,
            )
        };
        if check(ok).is_err() {
            return Cursor::default();
        }
        Cursor {
            in_viewport: out.viewport_has_value,
            x: out.viewport_x,
            y: out.viewport_y,
            visible: out.visible,
            blinking: out.blinking,
            wide_tail: out.wide_tail,
            style: CursorStyle::from_sys(out.visual_style),
        }
    }

    /// Visits every row of the current viewport.
    ///
    /// The closure runs while the iterators are positioned on the row, which is
    /// the only time cell data may be read.
    pub fn for_each_row<F>(&mut self, mut f: F)
    where
        F: FnMut(Row),
    {
        unsafe {
            let ok = sys::ghostty_render_state_get(
                self.raw,
                sys::GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                &mut self.rows as *mut _ as *mut c_void,
            );
            if check(ok).is_err() {
                return;
            }

            while sys::ghostty_render_state_row_iterator_next(self.rows) {
                let mut viewport_y = 0i32;
                let _ = sys::ghostty_render_state_row_get(
                    self.rows,
                    sys::GHOSTTY_RENDER_STATE_ROW_DATA_VIEWPORT_Y,
                    &mut viewport_y as *mut _ as *mut c_void,
                );

                let ok = sys::ghostty_render_state_row_get(
                    self.rows,
                    sys::GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    &mut self.cells as *mut _ as *mut c_void,
                );
                if check(ok).is_err() {
                    continue;
                }

                let mut row = Row {
                    cells: Vec::new(),
                    viewport_y,
                };

                while sys::ghostty_render_state_row_cells_next(self.cells) {
                    row.cells.push(read_cell(self.cells));
                }

                f(row);
            }
        }
    }

    /// Every image placement the terminal currently reports, in draw order.
    ///
    /// Sorted by layer and then z-index, which is the order a renderer wants:
    /// anything below the text first, so what is drawn above it covers what is
    /// underneath. The library reports them in storage order, which is not the
    /// same thing.
    ///
    /// Call this after [`RenderState::update`], in the same pass that reads the
    /// rows: the placements are only valid until the next update.
    pub fn images(&mut self) -> Vec<image::ImagePlacement> {
        self.images
            .placements(self.raw)
            .unwrap_or_else(|_| Vec::new())
    }
}

/// Reads the cell the `cells` iterator currently points at.
unsafe fn read_cell(cells: sys::GhosttyRenderStateRowCells) -> Cell {
    let mut style = sys::GhosttyStyle {
        fg_color: sys::GhosttyStyleColor {
            tag: sys::GHOSTTY_STYLE_COLOR_NONE,
            value: sys::GhosttyStyleColorValue { palette: 0 },
        },
        bg_color: sys::GhosttyStyleColor {
            tag: sys::GHOSTTY_STYLE_COLOR_NONE,
            value: sys::GhosttyStyleColorValue { palette: 0 },
        },
        underline_color: sys::GhosttyStyleColor {
            tag: sys::GHOSTTY_STYLE_COLOR_NONE,
            value: sys::GhosttyStyleColorValue { palette: 0 },
        },
        flags: 0,
        reserved: 0,
    };

    let _ = unsafe {
        sys::ghostty_render_state_row_cells_get(
            cells,
            sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
            &mut style as *mut _ as *mut c_void,
        )
    };

    let mut fg = sys::GhosttyColorRgb::default();
    let _ = unsafe {
        sys::ghostty_render_state_row_cells_get(
            cells,
            sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
            &mut fg as *mut _ as *mut c_void,
        )
    };

    let mut bg = sys::GhosttyColorRgb::default();
    let _ = unsafe {
        sys::ghostty_render_state_row_cells_get(
            cells,
            sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
            &mut bg as *mut _ as *mut c_void,
        )
    };

    let mut selected = false;
    let _ = unsafe {
        sys::ghostty_render_state_row_cells_get(
            cells,
            sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_SELECTED,
            &mut selected as *mut _ as *mut c_void,
        )
    };

    let mut buf = [0u8; 64];
    let len;
    let text = {
        let mut string = sys::GhosttyBuffer {
            ptr: buf.as_mut_ptr(),
            cap: buf.len(),
            len: 0,
        };
        let result = unsafe {
            sys::ghostty_render_state_row_cells_get(
                cells,
                sys::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
                &mut string as *mut _ as *mut c_void,
            )
        };
        len = string.len;
        if result == sys::GHOSTTY_SUCCESS && len > 0 && len <= buf.len() {
            String::from_utf8_lossy(&buf[..len]).into_owned()
        } else {
            String::new()
        }
    };

    let flags = style.flags;
    Cell {
        text,
        style: CellStyle {
            fg: Rgb::from_sys(fg),
            bg: Rgb::from_sys(bg),
            bold: flags & sys::GHOSTTY_STYLE_FLAG_BOLD != 0,
            italic: flags & sys::GHOSTTY_STYLE_FLAG_ITALIC != 0,
            faint: flags & sys::GHOSTTY_STYLE_FLAG_FAINT != 0,
            inverse: flags & sys::GHOSTTY_STYLE_FLAG_INVERSE != 0,
            invisible: flags & sys::GHOSTTY_STYLE_FLAG_INVISIBLE != 0,
            strikethrough: flags & sys::GHOSTTY_STYLE_FLAG_STRIKETHROUGH != 0,
            overline: flags & sys::GHOSTTY_STYLE_FLAG_OVERLINE != 0,
            underline: (flags & sys::GHOSTTY_STYLE_UNDERLINE_MASK)
                >> sys::GHOSTTY_STYLE_UNDERLINE_SHIFT
                != 0,
        },
        selected,
        wide_tail: false,
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        unsafe {
            sys::ghostty_render_state_row_cells_free(self.cells);
            sys::ghostty_render_state_row_iterator_free(self.rows);
            sys::ghostty_render_state_free(self.raw);
        }
    }
}

/// Modifier bits for [`KeyInput::mods`].
pub const MODS_SHIFT: u16 = sys::GHOSTTY_MODS_SHIFT;
pub const MODS_CTRL: u16 = sys::GHOSTTY_MODS_CTRL;
pub const MODS_ALT: u16 = sys::GHOSTTY_MODS_ALT;
pub const MODS_SUPER: u16 = sys::GHOSTTY_MODS_SUPER;

/// Key codes for [`KeyInput::key`]. Names follow the C enum.
mod image;
pub mod key {
    use vt_sys as sys;

    macro_rules! keys {
        ($($name:ident = $value:expr),* $(,)?) => {
            $(pub const $name: i32 = $value;)*
        };
    }

    keys! {
        UNIDENTIFIED = sys::GHOSTTY_KEY_UNIDENTIFIED,
        BACKQUOTE = sys::GHOSTTY_KEY_BACKQUOTE,
        BACKSLASH = sys::GHOSTTY_KEY_BACKSLASH,
        BRACKET_LEFT = sys::GHOSTTY_KEY_BRACKET_LEFT,
        BRACKET_RIGHT = sys::GHOSTTY_KEY_BRACKET_RIGHT,
        COMMA = sys::GHOSTTY_KEY_COMMA,
        DIGIT_0 = sys::GHOSTTY_KEY_DIGIT_0,
        DIGIT_1 = sys::GHOSTTY_KEY_DIGIT_1,
        DIGIT_2 = sys::GHOSTTY_KEY_DIGIT_2,
        DIGIT_3 = sys::GHOSTTY_KEY_DIGIT_3,
        DIGIT_4 = sys::GHOSTTY_KEY_DIGIT_4,
        DIGIT_5 = sys::GHOSTTY_KEY_DIGIT_5,
        DIGIT_6 = sys::GHOSTTY_KEY_DIGIT_6,
        DIGIT_7 = sys::GHOSTTY_KEY_DIGIT_7,
        DIGIT_8 = sys::GHOSTTY_KEY_DIGIT_8,
        DIGIT_9 = sys::GHOSTTY_KEY_DIGIT_9,
        EQUAL = sys::GHOSTTY_KEY_EQUAL,
        A = sys::GHOSTTY_KEY_A,
        B = sys::GHOSTTY_KEY_B,
        C = sys::GHOSTTY_KEY_C,
        D = sys::GHOSTTY_KEY_D,
        E = sys::GHOSTTY_KEY_E,
        F = sys::GHOSTTY_KEY_F,
        G = sys::GHOSTTY_KEY_G,
        H = sys::GHOSTTY_KEY_H,
        I = sys::GHOSTTY_KEY_I,
        J = sys::GHOSTTY_KEY_J,
        K = sys::GHOSTTY_KEY_K,
        L = sys::GHOSTTY_KEY_L,
        M = sys::GHOSTTY_KEY_M,
        N = sys::GHOSTTY_KEY_N,
        O = sys::GHOSTTY_KEY_O,
        P = sys::GHOSTTY_KEY_P,
        Q = sys::GHOSTTY_KEY_Q,
        R = sys::GHOSTTY_KEY_R,
        S = sys::GHOSTTY_KEY_S,
        T = sys::GHOSTTY_KEY_T,
        U = sys::GHOSTTY_KEY_U,
        V = sys::GHOSTTY_KEY_V,
        W = sys::GHOSTTY_KEY_W,
        X = sys::GHOSTTY_KEY_X,
        Y = sys::GHOSTTY_KEY_Y,
        Z = sys::GHOSTTY_KEY_Z,
        MINUS = sys::GHOSTTY_KEY_MINUS,
        PERIOD = sys::GHOSTTY_KEY_PERIOD,
        QUOTE = sys::GHOSTTY_KEY_QUOTE,
        SEMICOLON = sys::GHOSTTY_KEY_SEMICOLON,
        SLASH = sys::GHOSTTY_KEY_SLASH,
        ALT_LEFT = sys::GHOSTTY_KEY_ALT_LEFT,
        ALT_RIGHT = sys::GHOSTTY_KEY_ALT_RIGHT,
        BACKSPACE = sys::GHOSTTY_KEY_BACKSPACE,
        CAPS_LOCK = sys::GHOSTTY_KEY_CAPS_LOCK,
        CONTEXT_MENU = sys::GHOSTTY_KEY_CONTEXT_MENU,
        CONTROL_LEFT = sys::GHOSTTY_KEY_CONTROL_LEFT,
        CONTROL_RIGHT = sys::GHOSTTY_KEY_CONTROL_RIGHT,
        ENTER = sys::GHOSTTY_KEY_ENTER,
        META_LEFT = sys::GHOSTTY_KEY_META_LEFT,
        META_RIGHT = sys::GHOSTTY_KEY_META_RIGHT,
        SHIFT_LEFT = sys::GHOSTTY_KEY_SHIFT_LEFT,
        SHIFT_RIGHT = sys::GHOSTTY_KEY_SHIFT_RIGHT,
        SPACE = sys::GHOSTTY_KEY_SPACE,
        TAB = sys::GHOSTTY_KEY_TAB,
        DELETE = sys::GHOSTTY_KEY_DELETE,
        END = sys::GHOSTTY_KEY_END,
        HOME = sys::GHOSTTY_KEY_HOME,
        INSERT = sys::GHOSTTY_KEY_INSERT,
        PAGE_DOWN = sys::GHOSTTY_KEY_PAGE_DOWN,
        PAGE_UP = sys::GHOSTTY_KEY_PAGE_UP,
        ARROW_DOWN = sys::GHOSTTY_KEY_ARROW_DOWN,
        ARROW_LEFT = sys::GHOSTTY_KEY_ARROW_LEFT,
        ARROW_RIGHT = sys::GHOSTTY_KEY_ARROW_RIGHT,
        ARROW_UP = sys::GHOSTTY_KEY_ARROW_UP,
        ESCAPE = sys::GHOSTTY_KEY_ESCAPE,
        F1 = sys::GHOSTTY_KEY_F1,
        F2 = sys::GHOSTTY_KEY_F2,
        F3 = sys::GHOSTTY_KEY_F3,
        F4 = sys::GHOSTTY_KEY_F4,
        F5 = sys::GHOSTTY_KEY_F5,
        F6 = sys::GHOSTTY_KEY_F6,
        F7 = sys::GHOSTTY_KEY_F7,
        F8 = sys::GHOSTTY_KEY_F8,
        F9 = sys::GHOSTTY_KEY_F9,
        F10 = sys::GHOSTTY_KEY_F10,
        F11 = sys::GHOSTTY_KEY_F11,
        F12 = sys::GHOSTTY_KEY_F12,
    }
}

pub use image::*;
pub use key::*;
