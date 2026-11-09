//! How far a Kitty image moves the cursor.
//!
//! This matters more on Windows than anywhere else. The console behind a
//! pseudo-terminal implements no graphics protocol, so it cannot know an image
//! took rows; a terminal that does move the cursor past one ends up disagreeing
//! with the console by the height of every picture, and a shell that addresses
//! its input line by absolute row then writes into the picture. The `C=1`
//! parameter is what lets the two agree, so the library has to honour it — in a
//! single chunk and across several.

use vt::{RenderState, Terminal};

/// 8×60 RGB is 1440 bytes, which is 1920 base64 characters.
const DATA: usize = 1920;

fn payload() -> String {
    "A".repeat(DATA)
}

/// Feeds `seq` into a fresh terminal and returns how many rows the cursor moved.
fn rows_moved(seq: &[u8]) -> u16 {
    let mut term = Terminal::new(60, 20, None).expect("terminal");
    // The cell size decides how many rows the image covers; `C=1` has to hold
    // whether it covers one row or several.
    term.resize(60, 20, 8, 20);

    let mut state = RenderState::new().expect("state");
    state.update(term.raw()).expect("update");
    let before = state.cursor().y;

    term.feed(seq);
    state.update(term.raw()).expect("update");

    assert_eq!(
        state.images().len(),
        1,
        "the image has to be placed either way"
    );
    state.cursor().y.saturating_sub(before)
}

#[test]
fn without_the_flag_the_cursor_moves_past_the_image() {
    let seq = format!("\x1b_Ga=T,f=24,s=8,v=60,i=1;{}\x1b\\", payload());
    // The image is three rows; the cursor stops on its last one.
    assert_eq!(rows_moved(seq.as_bytes()), 2);
}

#[test]
fn the_flag_keeps_the_cursor_where_it_was() {
    let seq = format!("\x1b_Ga=T,f=24,s=8,v=60,i=1,C=1;{}\x1b\\", payload());
    assert_eq!(rows_moved(seq.as_bytes()), 0);
}

#[test]
fn the_flag_survives_a_chunked_transmission() {
    let data = payload();
    let (head, tail) = data.split_at(DATA / 2);

    let mut seq = format!("\x1b_Ga=T,f=24,s=8,v=60,i=1,C=1,m=1;{head}\x1b\\").into_bytes();
    seq.extend_from_slice(format!("\x1b_Gm=0;{tail}\x1b\\").as_bytes());
    assert_eq!(rows_moved(&seq), 0);
}

#[test]
fn the_flag_works_when_the_parameters_arrive_alone() {
    // `chafa` sends the parameters in a chunk of their own, with no payload and
    // therefore no semicolon, and the pixels in later chunks.
    let data = payload();
    let (head, tail) = data.split_at(DATA / 2);

    let mut seq = b"\x1b_Ga=T,f=24,s=8,v=60,i=1,C=1,m=1\x1b\\".to_vec();
    seq.extend_from_slice(format!("\x1b_Gm=1;{head}\x1b\\").as_bytes());
    seq.extend_from_slice(format!("\x1b_Gm=0;{tail}\x1b\\").as_bytes());
    assert_eq!(rows_moved(&seq), 0);
}
