//! Smoke tests for the raw C API binding.
//!
//! These run the real library: a terminal is fed VT bytes and the render state
//! is read back. They exist because the FFI layer is hand-written, so a wrong
//! struct layout or enum value shows up here rather than as a garbled screen.

use vt::{RenderState, Rgb, Scroll, Terminal};

#[test]
fn parses_text_into_cells() {
    let mut term = Terminal::new(20, 4, None).expect("terminal");
    term.feed(b"hello");

    let mut state = RenderState::new().expect("render state");
    state.update(term.raw()).expect("update");

    let mut first_row = None;
    state.for_each_row(|row| {
        if row.viewport_y == 0 {
            first_row = Some(row);
        }
    });

    let row = first_row.expect("first row");
    let text: String = row.cells.iter().map(|c| c.text.as_str()).collect();
    assert!(
        text.starts_with("hello"),
        "expected the row to start with the fed text, got {text:?}"
    );
}

#[test]
fn cursor_follows_typed_text() {
    let mut term = Terminal::new(20, 4, None).expect("terminal");
    term.feed(b"abc");

    let mut state = RenderState::new().expect("render state");
    state.update(term.raw()).expect("update");

    let cursor = state.cursor();
    assert!(cursor.in_viewport, "cursor should be inside the viewport");
    assert_eq!(cursor.x, 3, "cursor should sit after the three cells");
    assert_eq!(cursor.y, 0, "cursor should stay on the first row");
    assert!(cursor.visible, "cursor should be visible by default");
}

#[test]
fn size_matches_construction() {
    let term = Terminal::new(80, 24, None).expect("terminal");
    assert_eq!(term.size(), (80, 24));

    let mut state = RenderState::new().expect("render state");
    state.update(term.raw()).expect("update");
    assert_eq!(state.size(), (80, 24));
}

#[test]
fn colors_round_trip_through_the_terminal() {
    let mut term = Terminal::new(10, 2, None).expect("terminal");

    let mut palette = [Rgb::new(0, 0, 0); 256];
    palette[1] = Rgb::new(0xAA, 0x11, 0x22);
    term.set_colors(
        Rgb::new(0xEE, 0xEE, 0xEE),
        Rgb::new(0x10, 0x10, 0x10),
        None,
        &palette,
    );

    let mut state = RenderState::new().expect("render state");
    state.update(term.raw()).expect("update");

    let colors = state.colors();
    assert_eq!(colors.background, Rgb::new(0x10, 0x10, 0x10));
    assert_eq!(colors.foreground, Rgb::new(0xEE, 0xEE, 0xEE));
    assert_eq!(colors.palette[1], Rgb::new(0xAA, 0x11, 0x22));
}

#[test]
fn sgr_bold_reaches_the_cell_style() {
    let mut term = Terminal::new(20, 2, None).expect("terminal");
    term.feed(b"\x1b[1mB\x1b[0mn");

    let mut state = RenderState::new().expect("render state");
    state.update(term.raw()).expect("update");

    let mut cells = Vec::new();
    state.for_each_row(|row| {
        if row.viewport_y == 0 {
            cells = row.cells;
        }
    });

    assert!(
        cells.len() >= 2,
        "expected at least two cells, got {}",
        cells.len()
    );
    assert_eq!(cells[0].text, "B");
    assert!(cells[0].style.bold, "first cell should be bold");
    assert!(!cells[1].style.bold, "second cell should not be bold");
}

#[test]
fn wide_characters_occupy_two_cells() {
    let mut term = Terminal::new(20, 2, None).expect("terminal");
    term.feed("你好".as_bytes());

    let mut state = RenderState::new().expect("render state");
    state.update(term.raw()).expect("update");

    let mut cells = Vec::new();
    state.for_each_row(|row| {
        if row.viewport_y == 0 {
            cells = row.cells;
        }
    });

    assert_eq!(cells[0].text, "你");
    // The tail half of a wide glyph carries no text but still takes a column.
    assert_eq!(cells[1].text, "");
    assert_eq!(cells[2].text, "好");

    let cursor = state.cursor();
    assert_eq!(cursor.x, 4, "two wide glyphs advance four columns");
}

#[test]
fn scrollbar_reports_a_usable_range() {
    let mut term = Terminal::new(20, 3, None).expect("terminal");
    for i in 0..20 {
        term.feed(format!("line {i}\r\n").as_bytes());
    }

    let bar = term.scrollbar();
    assert!(
        bar.total >= bar.len,
        "total {} should cover the viewport",
        bar.total
    );
    assert_eq!(bar.len, 3, "viewport is three rows");

    term.scroll(Scroll::Top);
    let top = term.scrollbar();
    assert_eq!(top.offset, 0, "scrolling to the top should zero the offset");

    term.scroll(Scroll::Bottom);
    let bottom = term.scrollbar();
    assert_eq!(bottom.offset, bottom.total - bottom.len);
}

#[test]
fn write_pty_callback_receives_device_answers() {
    use std::sync::{Arc, Mutex};

    let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = Arc::clone(&seen);

    let mut term = Terminal::new(
        20,
        2,
        Some(Box::new(move |bytes: &[u8]| {
            sink.lock().unwrap().extend_from_slice(bytes);
        })),
    )
    .expect("terminal");

    // Primary device attributes: the terminal must answer with a reply.
    term.feed(b"\x1b[c");

    let answer = seen.lock().unwrap().clone();
    assert!(
        !answer.is_empty(),
        "the terminal should have answered the device attributes query"
    );
    assert_eq!(
        answer.first(),
        Some(&0x1b),
        "the reply should be an escape sequence"
    );
}

#[test]
fn selection_text_contains_the_fed_line() {
    let mut term = Terminal::new(30, 4, None).expect("terminal");
    term.feed(b"alpha beta");

    let text = term.select_all_text().expect("selection text");
    assert!(
        text.contains("alpha beta"),
        "selection should contain the fed text, got {text:?}"
    );
}

#[test]
fn key_encoding_produces_expected_sequences() {
    use vt::KeyInput;

    let mut term = Terminal::new(20, 2, None).expect("terminal");

    let enter = term.encode_key(KeyInput {
        key: vt::key::ENTER,
        mods: 0,
        utf8: Some("\r"),
    });
    assert_eq!(enter, b"\r", "Enter should encode as CR");

    let up = term.encode_key(KeyInput {
        key: vt::key::ARROW_UP,
        mods: 0,
        utf8: None,
    });
    assert_eq!(up, b"\x1b[A", "ArrowUp should encode as the CSI form");

    let ctrl_c = term.encode_key(KeyInput {
        key: vt::key::C,
        mods: vt::MODS_CTRL,
        utf8: Some("c"),
    });
    assert_eq!(ctrl_c, b"\x03", "Ctrl+C should encode as ETX");
}
