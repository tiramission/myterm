//! One terminal session: a ConPTY process, its reader thread, its writer
//! thread and the VT thread from [`crate::term`].

use std::cell::RefCell;
use std::io::{Read, Write};
use std::sync::mpsc::channel;
use std::sync::Arc;

use libghostty_vt::key;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::profiles::Profile;
use crate::term::{self, Cmd, Frame, Shared};

pub struct Session {
    cmd: std::sync::mpsc::Sender<Cmd>,
    shared: Arc<Shared>,
    master: Box<dyn MasterPty + Send>,
    child: RefCell<Box<dyn Child + Send + Sync>>,
}

impl Session {
    pub fn spawn(
        profile: &Profile,
        cols: u16,
        rows: u16,
        cell_w: u32,
        cell_h: u32,
    ) -> Result<Self, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("ConPTY: {e}"))?;

        let mut command = CommandBuilder::new(&profile.program);
        for arg in &profile.args {
            command.arg(arg);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("{}: {e}", profile.program))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("reader: {e}"))?;
        let mut writer = pair.master.take_writer().map_err(|e| format!("writer: {e}"))?;

        let (pty_tx, pty_write_rx) = channel::<Vec<u8>>();
        let (pty_read_tx, pty_rx) = channel::<Vec<u8>>();

        std::thread::Builder::new()
            .name("myterm-pty-read".into())
            .spawn(move || {
                let mut buf = [0_u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            log::debug!("pty reader: end of stream");
                            break;
                        }
                        Ok(n) => {
                            if pty_read_tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            log::debug!("pty reader: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        std::thread::Builder::new()
            .name("myterm-pty-write".into())
            .spawn(move || {
                while let Ok(bytes) = pty_write_rx.recv() {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
            })
            .map_err(|e| e.to_string())?;

        let vt_tx = pty_tx.clone();
        let (cmd, shared) = term::spawn(cols, rows, cell_w, cell_h, pty_rx, vt_tx);

        Ok(Self {
            cmd,
            shared,
            master: pair.master,
            child: RefCell::new(child),
        })
    }

    /// Text committed by the keyboard or the IME.
    pub fn send_text(&self, text: &str) {
        let _ = self.cmd.send(Cmd::Text(text.to_owned()));
    }

    /// A key that is not text: arrows, Enter, Tab, Ctrl+letter…
    pub fn send_key(&self, key: key::Key, mods: key::Mods) {
        let _ = self.cmd.send(Cmd::Key { key, mods });
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u32, cell_h: u32) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        let _ = self.cmd.send(Cmd::Resize {
            cols,
            rows,
            cell_w,
            cell_h,
        });
    }

    /// Extends the selection to (`col`, `row`), rows counted from the top of
    /// the viewport.
    pub fn select(&self, anchor: (u16, u32), focus: (u16, u32)) {
        let _ = self.cmd.send(Cmd::Select { anchor, focus });
    }

    pub fn select_all(&self) {
        let _ = self.cmd.send(Cmd::SelectAll);
    }

    pub fn clear_selection(&self) {
        let _ = self.cmd.send(Cmd::ClearSelection);
    }

    pub fn scroll(&self, delta: i32) {
        let _ = self.cmd.send(Cmd::Scroll(delta));
    }

    /// Searches the scrollback; `direction` 1 is the next (older) match.
    pub fn find(&self, needle: &str, direction: i8, skip: usize) {
        let _ = self.cmd.send(Cmd::Find {
            needle: needle.to_owned(),
            direction,
            skip,
        });
    }

    /// (matches, selected 1-based) of the last find.
    pub fn find_result(&self) -> Option<(usize, usize)> {
        self.shared.find.lock().ok()?.clone()
    }

    /// The selected text as of the last selection change (`""`: none).
    pub fn selection_text(&self) -> String {
        self.shared
            .selection
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }

    /// Polls the child; `Some(code)` once it exited (`-1` when unknown).
    pub fn poll_exit(&self) -> Option<i32> {
        let mut child = self.child.borrow_mut();
        match child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code() as i32),
            Ok(None) => None,
            Err(_) => Some(-1),
        }
    }

    /// Takes the newest frame, leaving the slot empty until the next one.
    pub fn take_frame(&self) -> Option<Arc<Frame>> {
        self.shared.latest.lock().ok()?.take()
    }

    pub fn title(&self) -> Option<String> {
        self.shared.title.lock().ok()?.clone()
    }

    #[allow(dead_code)]
    pub fn frame_ms(&self) -> f32 {
        self.shared.frame_ms.lock().map(|v| *v).unwrap_or(0.0)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_borrow_mut() {
            let _ = child.kill();
        }
    }
}
