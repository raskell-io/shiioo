//! A harness's own terminal UI under a pseudo-terminal, for a takeover.
//! Any number of viewers can watch it (each gets the recent screen output
//! first, so it can redraw) and type into it; the process belongs to the
//! takeover, not to a connection.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, watch};

/// Output kept for a viewer that attaches later: enough to redraw a
/// screen and some lines above it.
const SCROLLBACK: usize = 64 * 1024;

pub struct Terminal {
    writer: tokio::sync::Mutex<pty_process::OwnedWritePty>,
    output: broadcast::Sender<Arc<[u8]>>,
    scrollback: Mutex<VecDeque<u8>>,
    exited: watch::Receiver<bool>,
    /// The process group (the harness is its session leader).
    pgid: Option<u32>,
}

impl Terminal {
    /// Spawns `cmd` with a new terminal of `cols` × `rows` as its stdio.
    pub fn spawn(cmd: tokio::process::Command, cols: u16, rows: u16) -> std::io::Result<Arc<Self>> {
        let (pty, pts) = pty_process::open().map_err(std::io::Error::other)?;
        pty.resize(pty_process::Size::new(rows, cols))
            .map_err(std::io::Error::other)?;
        let mut child = pty_process::Command::from(cmd)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .spawn(pts)
            .map_err(std::io::Error::other)?;
        let pgid = child.id();
        let (mut reader, writer) = pty.into_split();
        let (output, _) = broadcast::channel(256);
        let (exited_tx, exited) = watch::channel(false);
        let terminal = Arc::new(Self {
            writer: tokio::sync::Mutex::new(writer),
            output,
            scrollback: Mutex::new(VecDeque::with_capacity(SCROLLBACK)),
            exited,
            pgid,
        });
        // Weak, so dropping the terminal (and with it the process group)
        // isn't held up by its own reader.
        let me = Arc::downgrade(&terminal);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 16 * 1024];
            // EOF, or EIO once the last process holding the terminal exits.
            while let Ok(n @ 1..) = reader.read(&mut buf).await {
                let Some(me) = me.upgrade() else { break };
                let chunk: Arc<[u8]> = Arc::from(&buf[..n]);
                if let Ok(mut back) = me.scrollback.lock() {
                    back.extend(chunk.iter());
                    let over = back.len().saturating_sub(SCROLLBACK);
                    back.drain(..over);
                }
                me.output.send(chunk).ok();
            }
            child.wait().await.ok();
            exited_tx.send_replace(true);
        });
        Ok(terminal)
    }

    /// The recent output (to redraw from) and the live output after it.
    pub fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Arc<[u8]>>) {
        // Under the lock, so no chunk falls between the copy and the
        // subscription.
        let back = self.scrollback.lock().ok();
        let rx = self.output.subscribe();
        let copy = back
            .map(|b| b.iter().copied().collect())
            .unwrap_or_default();
        (copy, rx)
    }

    /// Keystrokes from a viewer.
    pub async fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().await;
        w.write_all(bytes).await?;
        w.flush().await
    }

    pub async fn resize(&self, cols: u16, rows: u16) {
        let w = self.writer.lock().await;
        w.resize(pty_process::Size::new(rows.max(2), cols.max(2)))
            .ok();
    }

    /// Flips to `true` when the harness has exited.
    pub fn exited(&self) -> watch::Receiver<bool> {
        self.exited.clone()
    }

    pub fn has_exited(&self) -> bool {
        *self.exited.borrow()
    }

    /// Ends the harness: SIGTERM to its group, so it can save its session,
    /// then SIGKILL after `grace`.
    pub async fn stop(&self, grace: Duration) {
        let Some(pgid) = self.pgid else { return };
        if self.has_exited() {
            return;
        }
        super::signal_group(pgid, "TERM").await;
        let mut exited = self.exited();
        let done = tokio::time::timeout(grace, exited.wait_for(|e| *e))
            .await
            .is_ok();
        if !done {
            super::signal_group(pgid, "KILL").await;
            let mut exited = self.exited();
            tokio::time::timeout(Duration::from_secs(2), exited.wait_for(|e| *e))
                .await
                .ok();
        }
    }
}

impl Drop for Terminal {
    /// A takeover that goes away takes its process group with it.
    fn drop(&mut self) {
        if let Some(pgid) = self.pgid.filter(|_| !*self.exited.borrow()) {
            let _ = std::process::Command::new("kill")
                .args(["-s", "KILL", "--", &format!("-{pgid}")])
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
}
