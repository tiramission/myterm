//! Raw FFI declarations for libghostty-vt 1.0.0.
//!
//! Hand-written against the C headers in `vendor/include/ghostty`, which come
//! from the 1.0.0 release tarball. The library is statically linked, so every
//! declaration here is `extern "C"` with no import library indirection.
//!
//! Nothing in this crate is safe to call without the invariants documented on
//! each item; the `vt` crate wraps them.
#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::ffi::c_void;

/// `GhosttyResult`: the library returns one of these from every fallible call.
pub type GhosttyResult = i32;

pub const GHOSTTY_SUCCESS: GhosttyResult = 0;
pub const GHOSTTY_OUT_OF_MEMORY: GhosttyResult = -1;
pub const GHOSTTY_INVALID_VALUE: GhosttyResult = -2;
pub const GHOSTTY_OUT_OF_SPACE: GhosttyResult = -3;
pub const GHOSTTY_NO_VALUE: GhosttyResult = -4;
pub const GHOSTTY_IO_ERROR: GhosttyResult = -5;
pub const GHOSTTY_LIMIT_EXCEEDED: GhosttyResult = -6;
pub const GHOSTTY_REJECTED: GhosttyResult = -7;
pub const GHOSTTY_UNSUPPORTED: GhosttyResult = -8;

/// An opaque terminal. Owned by the caller, freed with [`ghostty_terminal_free`].
pub type GhosttyTerminal = *mut c_void;
/// An opaque render state.
pub type GhosttyRenderState = *mut c_void;
/// An opaque row iterator over a render state.
pub type GhosttyRenderStateRowIterator = *mut c_void;
/// An opaque cell iterator over one row.
pub type GhosttyRenderStateRowCells = *mut c_void;
/// An opaque key encoder.
pub type GhosttyKeyEncoder = *mut c_void;
/// An opaque key event.
pub type GhosttyKeyEvent = *mut c_void;
/// An opaque formatter.
pub type GhosttyFormatter = *mut c_void;
/// An opaque search.
pub type GhosttySearch = *mut c_void;
/// An opaque selection gesture.
pub type GhosttySelectionGesture = *mut c_void;
/// An opaque selection gesture event.
pub type GhosttySelectionGestureEvent = *mut c_void;

/// A selected range of cells.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttySelection {
    pub start: GhosttyGridRef,
    pub end: GhosttyGridRef,
    pub rectangle: bool,
}

/// `GhosttyAllocator`: pass a null pointer to use the library's own allocator.
pub type GhosttyAllocator = c_void;

/// A borrowed byte string: pointer plus length, no terminator.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyString {
    pub ptr: *const u8,
    pub len: usize,
}

/// A caller-owned byte buffer. On `GHOSTTY_OUT_OF_SPACE`, `len` holds the
/// capacity that would have been needed.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyBuffer {
    pub ptr: *mut u8,
    pub cap: usize,
    pub len: usize,
}

/// An 8-bit-per-channel color.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GhosttyColorRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// A 0–255 palette index.
pub type GhosttyColorPaletteIndex = u8;

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyStyleColorValue {
    pub palette: GhosttyColorPaletteIndex,
    pub rgb: GhosttyColorRgb,
}

/// A style color: a tag plus a three-byte value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyStyleColor {
    pub tag: u8,
    pub value: GhosttyStyleColorValue,
}

pub const GHOSTTY_STYLE_COLOR_NONE: u8 = 0;
pub const GHOSTTY_STYLE_COLOR_PALETTE: u8 = 1;
pub const GHOSTTY_STYLE_COLOR_RGB: u8 = 2;

pub type GhosttyStyleFlags = u16;

pub const GHOSTTY_STYLE_FLAG_BOLD: u16 = 1 << 0;
pub const GHOSTTY_STYLE_FLAG_ITALIC: u16 = 1 << 1;
pub const GHOSTTY_STYLE_FLAG_FAINT: u16 = 1 << 2;
pub const GHOSTTY_STYLE_FLAG_BLINK: u16 = 1 << 3;
pub const GHOSTTY_STYLE_FLAG_INVERSE: u16 = 1 << 4;
pub const GHOSTTY_STYLE_FLAG_INVISIBLE: u16 = 1 << 5;
pub const GHOSTTY_STYLE_FLAG_STRIKETHROUGH: u16 = 1 << 6;
pub const GHOSTTY_STYLE_FLAG_OVERLINE: u16 = 1 << 7;
pub const GHOSTTY_STYLE_UNDERLINE_SHIFT: u16 = 8;
pub const GHOSTTY_STYLE_UNDERLINE_MASK: u16 = 0x7 << GHOSTTY_STYLE_UNDERLINE_SHIFT;

/// A cell's full appearance.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyStyle {
    pub fg_color: GhosttyStyleColor,
    pub bg_color: GhosttyStyleColor,
    pub underline_color: GhosttyStyleColor,
    pub flags: GhosttyStyleFlags,
    pub reserved: u16,
}

/// A tagged union selecting how the viewport moves.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyTerminalScrollViewport {
    pub tag: i32,
    pub value: GhosttyTerminalScrollViewportValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyTerminalScrollViewportValue {
    pub delta: isize,
    pub row: usize,
    pub delta_prompt: isize,
    pub _padding: [u64; 2],
}

/// Scrollbar geometry for the viewport.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GhosttyTerminalScrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// A cell position, tagged by the space it is measured in.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyPointCoordinate {
    pub x: u16,
    pub y: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyPointValue {
    pub coordinate: GhosttyPointCoordinate,
    pub _padding: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyPoint {
    pub tag: i32,
    pub value: GhosttyPointValue,
}

pub const GHOSTTY_POINT_TAG_ACTIVE: i32 = 0;
pub const GHOSTTY_POINT_TAG_VIEWPORT: i32 = 1;
pub const GHOSTTY_POINT_TAG_SCREEN: i32 = 2;
pub const GHOSTTY_POINT_TAG_HISTORY: i32 = 3;

/// A handle to one cell in the terminal's storage.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyGridRef {
    pub node: *mut c_void,
    pub x: u16,
    pub y: u16,
}

/// A row's selected column range, local to that row.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyRenderStateRowSelection {
    pub size: usize,
    pub start_x: u16,
    pub end_x: u16,
}

impl Default for GhosttyRenderStateRowSelection {
    fn default() -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            start_x: 0,
            end_x: 0,
        }
    }
}

/// Everything the renderer needs to draw the cursor.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyRenderStateCursor {
    pub size: usize,
    pub viewport_has_value: bool,
    pub viewport_x: u16,
    pub viewport_y: u16,
    pub wide_tail: bool,
    pub visible: bool,
    pub blinking: bool,
    pub password_input: bool,
    pub visual_style: i32,
}

impl Default for GhosttyRenderStateCursor {
    fn default() -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            viewport_has_value: false,
            viewport_x: 0,
            viewport_y: 0,
            wide_tail: false,
            visible: false,
            blinking: false,
            password_input: false,
            visual_style: 0,
        }
    }
}

/// The render state's default colors and its 256-entry palette.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyRenderStateColors {
    pub size: usize,
    pub background: GhosttyColorRgb,
    pub foreground: GhosttyColorRgb,
    pub cursor: GhosttyColorRgb,
    pub cursor_has_value: bool,
    pub palette: [GhosttyColorRgb; 256],
}

impl Default for GhosttyRenderStateColors {
    fn default() -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            background: GhosttyColorRgb::default(),
            foreground: GhosttyColorRgb::default(),
            cursor: GhosttyColorRgb::default(),
            cursor_has_value: false,
            palette: [GhosttyColorRgb::default(); 256],
        }
    }
}

pub const GHOSTTY_RENDER_STATE_DATA_COLS: i32 = 1;
pub const GHOSTTY_RENDER_STATE_DATA_ROWS: i32 = 2;
pub const GHOSTTY_RENDER_STATE_DATA_DIRTY: i32 = 3;
pub const GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR: i32 = 4;
pub const GHOSTTY_RENDER_STATE_DATA_COLOR_BACKGROUND: i32 = 5;
pub const GHOSTTY_RENDER_STATE_DATA_COLOR_FOREGROUND: i32 = 6;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR: i32 = 18;
pub const GHOSTTY_RENDER_STATE_DATA_COLORS: i32 = 19;

pub const GHOSTTY_RENDER_STATE_ROW_DATA_CELLS: i32 = 3;
pub const GHOSTTY_RENDER_STATE_ROW_DATA_SELECTION: i32 = 4;
pub const GHOSTTY_RENDER_STATE_ROW_DATA_VIEWPORT_Y: i32 = 6;
pub const GHOSTTY_RENDER_STATE_ROW_DATA_ID: i32 = 7;

pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW: i32 = 1;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE: i32 = 2;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN: i32 = 3;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8: i32 = 9;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR: i32 = 5;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR: i32 = 6;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_SELECTED: i32 = 7;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_HAS_STYLING: i32 = 8;

pub const GHOSTTY_TERMINAL_DATA_COLS: i32 = 1;
pub const GHOSTTY_TERMINAL_DATA_ROWS: i32 = 2;
pub const GHOSTTY_TERMINAL_DATA_CURSOR_VISIBLE: i32 = 7;
pub const GHOSTTY_TERMINAL_DATA_SCROLLBAR: i32 = 9;
pub const GHOSTTY_TERMINAL_DATA_TITLE: i32 = 12;
pub const GHOSTTY_TERMINAL_DATA_TOTAL_ROWS: i32 = 14;
pub const GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS: i32 = 15;

pub const GHOSTTY_TERMINAL_OPT_USERDATA: i32 = 1;
pub const GHOSTTY_TERMINAL_OPT_WRITE_PTY: i32 = 2;
pub const GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND: i32 = 12;
pub const GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND: i32 = 13;
pub const GHOSTTY_TERMINAL_OPT_COLOR_CURSOR: i32 = 14;
pub const GHOSTTY_TERMINAL_OPT_COLOR_PALETTE: i32 = 15;
pub const GHOSTTY_TERMINAL_OPT_SELECTION: i32 = 22;

pub const GHOSTTY_TERMINAL_SCROLL_VIEWPORT_TOP: i32 = 0;
pub const GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM: i32 = 1;
pub const GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA: i32 = 2;

pub const GHOSTTY_KEY_ACTION_PRESS: i32 = 1;
pub const GHOSTTY_MODS_SHIFT: u16 = 1 << 0;
pub const GHOSTTY_MODS_CTRL: u16 = 1 << 1;
pub const GHOSTTY_MODS_ALT: u16 = 1 << 2;
pub const GHOSTTY_MODS_SUPER: u16 = 1 << 3;

pub const GHOSTTY_FORMATTER_FORMAT_PLAIN: i32 = 0;
pub const GHOSTTY_FORMATTER_OPT_EMIT: i32 = 1;
pub const GHOSTTY_FORMATTER_OPT_TRIM: i32 = 3;
pub const GHOSTTY_FORMATTER_OPT_SELECTION: i32 = 4;

pub const GHOSTTY_SEARCH_OPT_NEEDLE: i32 = 1;
pub const GHOSTTY_SEARCH_DATA_STATUS: i32 = 1;
pub const GHOSTTY_SEARCH_DATA_TOTAL_MATCHES: i32 = 3;
pub const GHOSTTY_SEARCH_DATA_SELECTED_INDEX: i32 = 4;
pub const GHOSTTY_SEARCH_DATA_SELECTED_MATCH: i32 = 5;
pub const GHOSTTY_SEARCH_STATUS_RUNNING: i32 = 0;
pub const GHOSTTY_SEARCH_STATUS_FEED_REQUIRED: i32 = 1;
pub const GHOSTTY_SEARCH_STATUS_COMPLETE: i32 = 2;

pub const GHOSTTY_CURSOR_STYLE_BAR: i32 = 0;
pub const GHOSTTY_CURSOR_STYLE_BLOCK: i32 = 1;
pub const GHOSTTY_CURSOR_STYLE_UNDERLINE: i32 = 2;
pub const GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW: i32 = 3;

unsafe extern "C" {
    // ---- Core ----
    pub fn ghostty_sized_init(ptr: *mut c_void, size: usize);

    // ---- Terminal ----
    pub fn ghostty_terminal_new(
        allocator: *const GhosttyAllocator,
        terminal: *mut GhosttyTerminal,
        cols: u16,
        rows: u16,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_free(terminal: GhosttyTerminal);
    pub fn ghostty_terminal_resize(
        terminal: GhosttyTerminal,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_set(
        terminal: GhosttyTerminal,
        option: i32,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_vt_write(terminal: GhosttyTerminal, data: *const u8, len: usize);
    pub fn ghostty_terminal_scroll_viewport(
        terminal: GhosttyTerminal,
        behavior: GhosttyTerminalScrollViewport,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_get(
        terminal: GhosttyTerminal,
        data: i32,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_grid_ref(
        terminal: GhosttyTerminal,
        point: GhosttyPoint,
        out_ref: *mut GhosttyGridRef,
    ) -> GhosttyResult;

    // ---- Selection ----
    pub fn ghostty_terminal_select_all(
        terminal: GhosttyTerminal,
        out_selection: *mut GhosttySelection,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_selection_ordered(
        terminal: GhosttyTerminal,
        selection: *const GhosttySelection,
        desired: i32,
        out_selection: *mut GhosttySelection,
    ) -> GhosttyResult;

    // ---- Grid refs ----
    pub fn ghostty_grid_ref_graphemes(
        grid_ref: *const GhosttyGridRef,
        buf: *mut u32,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_grid_ref_style(
        grid_ref: *const GhosttyGridRef,
        out_style: *mut GhosttyStyle,
    ) -> GhosttyResult;

    // ---- Render state ----
    pub fn ghostty_render_state_new(
        allocator: *const GhosttyAllocator,
        state: *mut GhosttyRenderState,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_free(state: GhosttyRenderState);
    pub fn ghostty_render_state_update(
        state: GhosttyRenderState,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_get(
        state: GhosttyRenderState,
        data: i32,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_iterator_new(
        allocator: *const GhosttyAllocator,
        out_iterator: *mut GhosttyRenderStateRowIterator,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_iterator_free(iterator: GhosttyRenderStateRowIterator);
    pub fn ghostty_render_state_row_iterator_next(iterator: GhosttyRenderStateRowIterator) -> bool;
    pub fn ghostty_render_state_row_get(
        iterator: GhosttyRenderStateRowIterator,
        data: i32,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_cells_new(
        allocator: *const GhosttyAllocator,
        out_cells: *mut GhosttyRenderStateRowCells,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_cells_free(cells: GhosttyRenderStateRowCells);
    pub fn ghostty_render_state_row_cells_next(cells: GhosttyRenderStateRowCells) -> bool;
    pub fn ghostty_render_state_row_cells_get(
        cells: GhosttyRenderStateRowCells,
        data: i32,
        out: *mut c_void,
    ) -> GhosttyResult;

    // ---- Key encoding ----
    pub fn ghostty_key_encoder_new(
        allocator: *const GhosttyAllocator,
        encoder: *mut GhosttyKeyEncoder,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_free(encoder: GhosttyKeyEncoder);
    pub fn ghostty_key_encoder_set_from_terminal(
        encoder: GhosttyKeyEncoder,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_encode(
        encoder: GhosttyKeyEncoder,
        event: GhosttyKeyEvent,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_new(
        allocator: *const GhosttyAllocator,
        event: *mut GhosttyKeyEvent,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_free(event: GhosttyKeyEvent);
    pub fn ghostty_key_event_set_action(event: GhosttyKeyEvent, action: i32) -> GhosttyResult;
    pub fn ghostty_key_event_set_key(event: GhosttyKeyEvent, key: i32) -> GhosttyResult;
    pub fn ghostty_key_event_set_mods(event: GhosttyKeyEvent, mods: u16) -> GhosttyResult;
    pub fn ghostty_key_event_set_utf8(event: GhosttyKeyEvent, utf8: GhosttyString)
    -> GhosttyResult;

    // ---- Formatter ----
    pub fn ghostty_formatter_terminal_new(
        allocator: *const GhosttyAllocator,
        formatter: *mut GhosttyFormatter,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_free(formatter: GhosttyFormatter);
    pub fn ghostty_formatter_set(
        formatter: GhosttyFormatter,
        option: i32,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_format_alloc(
        formatter: GhosttyFormatter,
        allocator: *const GhosttyAllocator,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> GhosttyResult;

    // ---- Search ----
    pub fn ghostty_search_new(
        allocator: *const GhosttyAllocator,
        out: *mut GhosttySearch,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_search_free(search: GhosttySearch);
    pub fn ghostty_search_set(
        search: GhosttySearch,
        option: i32,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_search_run(search: GhosttySearch) -> GhosttyResult;
    pub fn ghostty_search_select_next(search: GhosttySearch) -> GhosttyResult;
    pub fn ghostty_search_select_prev(search: GhosttySearch) -> GhosttyResult;
    pub fn ghostty_search_get(search: GhosttySearch, data: i32, out: *mut c_void) -> GhosttyResult;

    // ---- Memory ----
    pub fn ghostty_free(allocator: *const GhosttyAllocator, ptr: *mut u8, len: usize);
}

// ---- Key codes (generated from the 1.0.0 headers) ----
pub const GHOSTTY_KEY_UNIDENTIFIED: i32 = 0;
pub const GHOSTTY_KEY_BACKQUOTE: i32 = 1;
pub const GHOSTTY_KEY_BACKSLASH: i32 = 2;
pub const GHOSTTY_KEY_BRACKET_LEFT: i32 = 3;
pub const GHOSTTY_KEY_BRACKET_RIGHT: i32 = 4;
pub const GHOSTTY_KEY_COMMA: i32 = 5;
pub const GHOSTTY_KEY_DIGIT_0: i32 = 6;
pub const GHOSTTY_KEY_DIGIT_1: i32 = 7;
pub const GHOSTTY_KEY_DIGIT_2: i32 = 8;
pub const GHOSTTY_KEY_DIGIT_3: i32 = 9;
pub const GHOSTTY_KEY_DIGIT_4: i32 = 10;
pub const GHOSTTY_KEY_DIGIT_5: i32 = 11;
pub const GHOSTTY_KEY_DIGIT_6: i32 = 12;
pub const GHOSTTY_KEY_DIGIT_7: i32 = 13;
pub const GHOSTTY_KEY_DIGIT_8: i32 = 14;
pub const GHOSTTY_KEY_DIGIT_9: i32 = 15;
pub const GHOSTTY_KEY_EQUAL: i32 = 16;
pub const GHOSTTY_KEY_INTL_BACKSLASH: i32 = 17;
pub const GHOSTTY_KEY_INTL_RO: i32 = 18;
pub const GHOSTTY_KEY_INTL_YEN: i32 = 19;
pub const GHOSTTY_KEY_A: i32 = 20;
pub const GHOSTTY_KEY_B: i32 = 21;
pub const GHOSTTY_KEY_C: i32 = 22;
pub const GHOSTTY_KEY_D: i32 = 23;
pub const GHOSTTY_KEY_E: i32 = 24;
pub const GHOSTTY_KEY_F: i32 = 25;
pub const GHOSTTY_KEY_G: i32 = 26;
pub const GHOSTTY_KEY_H: i32 = 27;
pub const GHOSTTY_KEY_I: i32 = 28;
pub const GHOSTTY_KEY_J: i32 = 29;
pub const GHOSTTY_KEY_K: i32 = 30;
pub const GHOSTTY_KEY_L: i32 = 31;
pub const GHOSTTY_KEY_M: i32 = 32;
pub const GHOSTTY_KEY_N: i32 = 33;
pub const GHOSTTY_KEY_O: i32 = 34;
pub const GHOSTTY_KEY_P: i32 = 35;
pub const GHOSTTY_KEY_Q: i32 = 36;
pub const GHOSTTY_KEY_R: i32 = 37;
pub const GHOSTTY_KEY_S: i32 = 38;
pub const GHOSTTY_KEY_T: i32 = 39;
pub const GHOSTTY_KEY_U: i32 = 40;
pub const GHOSTTY_KEY_V: i32 = 41;
pub const GHOSTTY_KEY_W: i32 = 42;
pub const GHOSTTY_KEY_X: i32 = 43;
pub const GHOSTTY_KEY_Y: i32 = 44;
pub const GHOSTTY_KEY_Z: i32 = 45;
pub const GHOSTTY_KEY_MINUS: i32 = 46;
pub const GHOSTTY_KEY_PERIOD: i32 = 47;
pub const GHOSTTY_KEY_QUOTE: i32 = 48;
pub const GHOSTTY_KEY_SEMICOLON: i32 = 49;
pub const GHOSTTY_KEY_SLASH: i32 = 50;
pub const GHOSTTY_KEY_ALT_LEFT: i32 = 51;
pub const GHOSTTY_KEY_ALT_RIGHT: i32 = 52;
pub const GHOSTTY_KEY_BACKSPACE: i32 = 53;
pub const GHOSTTY_KEY_CAPS_LOCK: i32 = 54;
pub const GHOSTTY_KEY_CONTEXT_MENU: i32 = 55;
pub const GHOSTTY_KEY_CONTROL_LEFT: i32 = 56;
pub const GHOSTTY_KEY_CONTROL_RIGHT: i32 = 57;
pub const GHOSTTY_KEY_ENTER: i32 = 58;
pub const GHOSTTY_KEY_META_LEFT: i32 = 59;
pub const GHOSTTY_KEY_META_RIGHT: i32 = 60;
pub const GHOSTTY_KEY_SHIFT_LEFT: i32 = 61;
pub const GHOSTTY_KEY_SHIFT_RIGHT: i32 = 62;
pub const GHOSTTY_KEY_SPACE: i32 = 63;
pub const GHOSTTY_KEY_TAB: i32 = 64;
pub const GHOSTTY_KEY_CONVERT: i32 = 65;
pub const GHOSTTY_KEY_KANA_MODE: i32 = 66;
pub const GHOSTTY_KEY_NON_CONVERT: i32 = 67;
pub const GHOSTTY_KEY_DELETE: i32 = 68;
pub const GHOSTTY_KEY_END: i32 = 69;
pub const GHOSTTY_KEY_HELP: i32 = 70;
pub const GHOSTTY_KEY_HOME: i32 = 71;
pub const GHOSTTY_KEY_INSERT: i32 = 72;
pub const GHOSTTY_KEY_PAGE_DOWN: i32 = 73;
pub const GHOSTTY_KEY_PAGE_UP: i32 = 74;
pub const GHOSTTY_KEY_ARROW_DOWN: i32 = 75;
pub const GHOSTTY_KEY_ARROW_LEFT: i32 = 76;
pub const GHOSTTY_KEY_ARROW_RIGHT: i32 = 77;
pub const GHOSTTY_KEY_ARROW_UP: i32 = 78;
pub const GHOSTTY_KEY_NUM_LOCK: i32 = 79;
pub const GHOSTTY_KEY_NUMPAD_0: i32 = 80;
pub const GHOSTTY_KEY_NUMPAD_1: i32 = 81;
pub const GHOSTTY_KEY_NUMPAD_2: i32 = 82;
pub const GHOSTTY_KEY_NUMPAD_3: i32 = 83;
pub const GHOSTTY_KEY_NUMPAD_4: i32 = 84;
pub const GHOSTTY_KEY_NUMPAD_5: i32 = 85;
pub const GHOSTTY_KEY_NUMPAD_6: i32 = 86;
pub const GHOSTTY_KEY_NUMPAD_7: i32 = 87;
pub const GHOSTTY_KEY_NUMPAD_8: i32 = 88;
pub const GHOSTTY_KEY_NUMPAD_9: i32 = 89;
pub const GHOSTTY_KEY_NUMPAD_ADD: i32 = 90;
pub const GHOSTTY_KEY_NUMPAD_BACKSPACE: i32 = 91;
pub const GHOSTTY_KEY_NUMPAD_CLEAR: i32 = 92;
pub const GHOSTTY_KEY_NUMPAD_CLEAR_ENTRY: i32 = 93;
pub const GHOSTTY_KEY_NUMPAD_COMMA: i32 = 94;
pub const GHOSTTY_KEY_NUMPAD_DECIMAL: i32 = 95;
pub const GHOSTTY_KEY_NUMPAD_DIVIDE: i32 = 96;
pub const GHOSTTY_KEY_NUMPAD_ENTER: i32 = 97;
pub const GHOSTTY_KEY_NUMPAD_EQUAL: i32 = 98;
pub const GHOSTTY_KEY_NUMPAD_MEMORY_ADD: i32 = 99;
pub const GHOSTTY_KEY_NUMPAD_MEMORY_CLEAR: i32 = 100;
pub const GHOSTTY_KEY_NUMPAD_MEMORY_RECALL: i32 = 101;
pub const GHOSTTY_KEY_NUMPAD_MEMORY_STORE: i32 = 102;
pub const GHOSTTY_KEY_NUMPAD_MEMORY_SUBTRACT: i32 = 103;
pub const GHOSTTY_KEY_NUMPAD_MULTIPLY: i32 = 104;
pub const GHOSTTY_KEY_NUMPAD_PAREN_LEFT: i32 = 105;
pub const GHOSTTY_KEY_NUMPAD_PAREN_RIGHT: i32 = 106;
pub const GHOSTTY_KEY_NUMPAD_SUBTRACT: i32 = 107;
pub const GHOSTTY_KEY_NUMPAD_SEPARATOR: i32 = 108;
pub const GHOSTTY_KEY_NUMPAD_UP: i32 = 109;
pub const GHOSTTY_KEY_NUMPAD_DOWN: i32 = 110;
pub const GHOSTTY_KEY_NUMPAD_RIGHT: i32 = 111;
pub const GHOSTTY_KEY_NUMPAD_LEFT: i32 = 112;
pub const GHOSTTY_KEY_NUMPAD_BEGIN: i32 = 113;
pub const GHOSTTY_KEY_NUMPAD_HOME: i32 = 114;
pub const GHOSTTY_KEY_NUMPAD_END: i32 = 115;
pub const GHOSTTY_KEY_NUMPAD_INSERT: i32 = 116;
pub const GHOSTTY_KEY_NUMPAD_DELETE: i32 = 117;
pub const GHOSTTY_KEY_NUMPAD_PAGE_UP: i32 = 118;
pub const GHOSTTY_KEY_NUMPAD_PAGE_DOWN: i32 = 119;
pub const GHOSTTY_KEY_ESCAPE: i32 = 120;
pub const GHOSTTY_KEY_F1: i32 = 121;
pub const GHOSTTY_KEY_F2: i32 = 122;
pub const GHOSTTY_KEY_F3: i32 = 123;
pub const GHOSTTY_KEY_F4: i32 = 124;
pub const GHOSTTY_KEY_F5: i32 = 125;
pub const GHOSTTY_KEY_F6: i32 = 126;
pub const GHOSTTY_KEY_F7: i32 = 127;
pub const GHOSTTY_KEY_F8: i32 = 128;
pub const GHOSTTY_KEY_F9: i32 = 129;
pub const GHOSTTY_KEY_F10: i32 = 130;
pub const GHOSTTY_KEY_F11: i32 = 131;
pub const GHOSTTY_KEY_F12: i32 = 132;
pub const GHOSTTY_KEY_F13: i32 = 133;
pub const GHOSTTY_KEY_F14: i32 = 134;
pub const GHOSTTY_KEY_F15: i32 = 135;
pub const GHOSTTY_KEY_F16: i32 = 136;
pub const GHOSTTY_KEY_F17: i32 = 137;
pub const GHOSTTY_KEY_F18: i32 = 138;
pub const GHOSTTY_KEY_F19: i32 = 139;
pub const GHOSTTY_KEY_F20: i32 = 140;
pub const GHOSTTY_KEY_F21: i32 = 141;
pub const GHOSTTY_KEY_F22: i32 = 142;
pub const GHOSTTY_KEY_F23: i32 = 143;
pub const GHOSTTY_KEY_F24: i32 = 144;
pub const GHOSTTY_KEY_F25: i32 = 145;
pub const GHOSTTY_KEY_FN: i32 = 146;
pub const GHOSTTY_KEY_FN_LOCK: i32 = 147;
pub const GHOSTTY_KEY_PRINT_SCREEN: i32 = 148;
pub const GHOSTTY_KEY_SCROLL_LOCK: i32 = 149;
pub const GHOSTTY_KEY_PAUSE: i32 = 150;
pub const GHOSTTY_KEY_BROWSER_BACK: i32 = 151;
pub const GHOSTTY_KEY_BROWSER_FAVORITES: i32 = 152;
pub const GHOSTTY_KEY_BROWSER_FORWARD: i32 = 153;
pub const GHOSTTY_KEY_BROWSER_HOME: i32 = 154;
pub const GHOSTTY_KEY_BROWSER_REFRESH: i32 = 155;
pub const GHOSTTY_KEY_BROWSER_SEARCH: i32 = 156;
pub const GHOSTTY_KEY_BROWSER_STOP: i32 = 157;
pub const GHOSTTY_KEY_EJECT: i32 = 158;
pub const GHOSTTY_KEY_LAUNCH_APP_1: i32 = 159;
pub const GHOSTTY_KEY_LAUNCH_APP_2: i32 = 160;
pub const GHOSTTY_KEY_LAUNCH_MAIL: i32 = 161;
pub const GHOSTTY_KEY_MEDIA_PLAY_PAUSE: i32 = 162;
pub const GHOSTTY_KEY_MEDIA_SELECT: i32 = 163;
pub const GHOSTTY_KEY_MEDIA_STOP: i32 = 164;
pub const GHOSTTY_KEY_MEDIA_TRACK_NEXT: i32 = 165;
pub const GHOSTTY_KEY_MEDIA_TRACK_PREVIOUS: i32 = 166;
pub const GHOSTTY_KEY_POWER: i32 = 167;
pub const GHOSTTY_KEY_SLEEP: i32 = 168;
pub const GHOSTTY_KEY_AUDIO_VOLUME_DOWN: i32 = 169;
pub const GHOSTTY_KEY_AUDIO_VOLUME_MUTE: i32 = 170;
pub const GHOSTTY_KEY_AUDIO_VOLUME_UP: i32 = 171;
pub const GHOSTTY_KEY_WAKE_UP: i32 = 172;
pub const GHOSTTY_KEY_COPY: i32 = 173;
pub const GHOSTTY_KEY_CUT: i32 = 174;
pub const GHOSTTY_KEY_PASTE: i32 = 175;
