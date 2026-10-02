//! Integrated terminal: real PTYs (portable-pty; ConPTY on Windows) streamed to xterm.js.
//!
//! Output is sent as raw bytes over a Tauri channel (no JSON/base64 encoding); a dedicated
//! reader thread per terminal reads in 64 KiB chunks so bursts arrive as few large messages.

use crate::error::{Error, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};

struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct Terminals {
    sessions: Arc<Mutex<HashMap<u32, Session>>>,
    next_id: AtomicU32,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 }
}

impl Terminals {
    /// Spawn a shell in `cwd`. `on_data` receives output bytes; `on_exit` is called once
    /// with the terminal id when the shell exits.
    pub fn spawn(
        &self,
        shell: &str,
        cwd: &Path,
        cols: u16,
        rows: u16,
        on_data: Channel<InvokeResponseBody>,
        on_exit: impl FnOnce(u32) + Send + 'static,
    ) -> Result<u32> {
        let pair = native_pty_system().openpty(size(cols, rows)).map_err(|e| Error::msg(e.to_string()))?;
        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "PiLunch");
        if let Some(path) = crate::process::login_path() {
            cmd.env("PATH", path);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| Error::msg(format!("cannot start {shell}: {e}")))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| Error::msg(e.to_string()))?;
        let writer = pair.master.take_writer().map_err(|e| Error::msg(e.to_string()))?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.sessions.lock().unwrap().insert(id, Session { writer, master: pair.master, child });

        let sessions = self.sessions.clone();
        std::thread::Builder::new()
            .name(format!("pty-{id}"))
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if on_data.send(InvokeResponseBody::Raw(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                    }
                }
                if let Some(mut s) = sessions.lock().unwrap().remove(&id) {
                    let _ = s.child.kill();
                    let _ = s.child.wait();
                }
                on_exit(id);
            })
            .map_err(|e| Error::msg(e.to_string()))?;
        Ok(id)
    }

    pub fn write(&self, id: u32, data: &[u8]) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        let s = sessions.get_mut(&id).ok_or_else(|| Error::msg("terminal not found"))?;
        s.writer.write_all(data)?;
        s.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let s = sessions.get(&id).ok_or_else(|| Error::msg("terminal not found"))?;
        s.master.resize(size(cols, rows)).map_err(|e| Error::msg(e.to_string()))
    }

    pub fn kill(&self, id: u32) {
        // Killing the child closes the PTY; the reader thread then cleans up.
        if let Some(s) = self.sessions.lock().unwrap().get_mut(&id) {
            let _ = s.child.kill();
        }
    }

    pub fn kill_all(&self) {
        for s in self.sessions.lock().unwrap().values_mut() {
            let _ = s.child.kill();
        }
    }
}
