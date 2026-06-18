use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PtyError {
    #[error("failed to open PTY: {0}")]
    Open(String),
    #[error("failed to spawn shell: {0}")]
    Spawn(String),
    #[error("failed to clone PTY reader: {0}")]
    Reader(String),
    #[error("failed to take PTY writer: {0}")]
    Writer(String),
    #[error("failed to wait for child process: {0}")]
    Wait(String),
}

/// A live PTY with a spawned shell. Call `take_reader`/`take_writer` to move
/// the I/O ends into threads; the child and master are kept here until drop.
pub struct PtyHandle {
    // Kept alive so the PTY fd does not close while the handle exists.
    _master: Box<dyn portable_pty::MasterPty + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    reader: Option<Box<dyn Read + Send>>,
    writer: Option<Box<dyn Write + Send>>,
}

impl PtyHandle {
    /// Spawn `$SHELL` (fallback `/bin/bash`) in a new PTY of the given size.
    pub fn spawn(cols: u16, rows: u16) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Open(e.to_string()))?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let cmd = CommandBuilder::new(&shell);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;

        // The child has inherited the slave fds; drop the slave handle now.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Reader(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Writer(e.to_string()))?;

        Ok(Self {
            _master: pair.master,
            child,
            reader: Some(reader),
            writer: Some(writer),
        })
    }

    /// Take the read end of the PTY. Panics if called more than once.
    pub fn take_reader(&mut self) -> Box<dyn Read + Send> {
        self.reader.take().expect("PTY reader already taken")
    }

    /// Take the write end of the PTY. Panics if called more than once.
    pub fn take_writer(&mut self) -> Box<dyn Write + Send> {
        self.writer.take().expect("PTY writer already taken")
    }
}
