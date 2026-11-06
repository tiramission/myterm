//! M0 check: ConPTY → libghostty-vt → frame data, without the UI.
//!
//! Runs on Windows only (ConPTY).

use std::io::Read;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::{RenderState, Terminal};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

#[test]
fn conpty_output_reaches_the_terminal_grid() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    let mut command = CommandBuilder::new("pwsh.exe");
    command.arg("-NoLogo");
    command.arg("-NoProfile");
    command.arg("-Command");
    command.arg("Write-Output 'myterm-m0-hello'; exit");
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0_u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut terminal = Terminal::new(80, 24).unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => terminal.vt_write(&chunk),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if text_of(&terminal).contains("myterm-m0-hello") {
            break;
        }
    }

    let _ = child.kill();
    let text = text_of(&terminal);
    assert!(
        text.contains("myterm-m0-hello"),
        "the shell's output did not reach the grid:\n{text}"
    );
    assert_eq!(terminal.cols().unwrap(), 80);
    assert_eq!(terminal.rows().unwrap(), 24);
}

fn text_of(terminal: &Terminal<'static, 'static>) -> String {
    let mut state = RenderState::new().unwrap();
    let mut rows = RowIterator::new().unwrap();
    let mut cells = CellIterator::new().unwrap();
    let snapshot = state.update(terminal).unwrap();
    let mut out = String::new();
    let mut buffer = String::new();
    let mut row_it = rows.update(&snapshot).unwrap();
    while let Some(row) = row_it.next() {
        let mut cell_it = cells.update(row).unwrap();
        while let Some(cell) = cell_it.next() {
            if cell.graphemes_len().unwrap() > 0 {
                cell.graphemes_utf8(&mut buffer).unwrap();
                out.push_str(&buffer);
            } else {
                out.push(' ');
            }
        }
        out.push('\n');
    }
    out
}
