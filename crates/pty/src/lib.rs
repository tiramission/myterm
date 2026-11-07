//! A ConPTY process plus its reader and writer threads.
//!
//! Bytes arrive on a channel so the UI thread can drain them without blocking,
//! and writes go through a second channel so the UI never waits on the pipe.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// What to start in a pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
}

/// A running child process attached to a pseudo console.
pub struct Pty {
    write_tx: Sender<Vec<u8>>,
    read_rx: Receiver<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Set once the reader thread saw end of stream.
    eof: bool,
}

impl Pty {
    /// Starts `command` at `cols`×`rows`.
    pub fn spawn(command: &Command, cols: u16, rows: u16) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("ConPTY: {e}"))?;

        let mut builder = CommandBuilder::new(&command.program);
        for arg in &command.args {
            builder.arg(arg);
        }
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");

        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| format!("{}: {e}", command.program))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("reader: {e}"))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("writer: {e}"))?;

        let (write_tx, write_rx) = channel::<Vec<u8>>();
        let (read_tx, read_rx) = channel::<Vec<u8>>();

        std::thread::Builder::new()
            .name("myterm-pty-read".into())
            .spawn(move || {
                let mut buf = [0_u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if read_tx.send(buf[..n].to_vec()).is_err() {
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
                while let Ok(bytes) = write_rx.recv() {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
            })
            .map_err(|e| e.to_string())?;

        Ok(Self {
            write_tx,
            read_rx,
            master: pair.master,
            child,
            eof: false,
        })
    }

    /// Queues bytes for the child.
    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let _ = self.write_tx.send(bytes.to_vec());
    }

    /// Drains everything the child produced since the last call.
    ///
    /// Returns the bytes and whether the stream ended.
    pub fn drain(&mut self) -> (Vec<u8>, bool) {
        let mut out = Vec::new();
        if self.eof {
            return (out, true);
        }
        loop {
            match self.read_rx.try_recv() {
                Ok(chunk) => out.extend_from_slice(&chunk),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.eof = true;
                    break;
                }
            }
        }
        (out, self.eof)
    }

    /// Resizes the pseudo console.
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// Polls the child; `Some(code)` once it exited.
    pub fn exit_code(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code() as i32),
            Ok(None) => None,
            Err(_) => Some(-1),
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
