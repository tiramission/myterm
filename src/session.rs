//! One terminal session: a ConPTY process, its reader thread, its writer
//! thread and the VT thread from [`crate::term`].

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::Arc;

use libghostty_vt::key;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::term::{self, Cmd, Frame, Shared};

pub struct Session {
    cmd: std::sync::mpsc::Sender<Cmd>,
    shared: Arc<Shared>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    pub profile: String,
}

impl Session {
    pub fn spawn(cols: u16, rows: u16, cell_w: u32, cell_h: u32) -> Result<Self, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("ConPTY: {e}"))?;

        let (program, profile) = pick_shell();
        let mut command = CommandBuilder::new(&program);
        command.arg("-NoLogo");
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("{program}: {e}"))?;
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
                            log::warn!("pty reader: end of stream");
                            break;
                        }
                        Ok(n) => {
                            if pty_read_tx.send(buf[..n].to_vec()).is_err() {
                                log::warn!("pty reader: vt thread gone");
                                break;
                            }
                        }
                        Err(e) => {
                            log::warn!("pty reader: {e}");
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
            child,
            profile,
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

    /// Takes the newest frame, leaving the slot empty until the next one.
    pub fn take_frame(&self) -> Option<Arc<Frame>> {
        self.shared.latest.lock().ok()?.take()
    }

    pub fn title(&self) -> Option<String> {
        self.shared.title.lock().ok()?.clone()
    }

    pub fn frame_ms(&self) -> f32 {
        self.shared.frame_ms.lock().map(|v| *v).unwrap_or(0.0)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// The shell a new session starts: PowerShell 7 if it is installed, the
/// Windows PowerShell otherwise.
fn pick_shell() -> (String, String) {
    if let Some(path) = find_on_path("pwsh.exe") {
        (path.to_string_lossy().into_owned(), "PowerShell 7".to_owned())
    } else {
        ("powershell.exe".to_owned(), "Windows PowerShell".to_owned())
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}
