use agentd::NaiveDetector;
use pty::PtyHandle;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("PTY error: {0}")]
    Pty(#[from] pty::PtyError),
    #[error("socket error: {0}")]
    Socket(#[from] std::io::Error),
}

/// Run the server: open a PTY, bind a Unix socket, accept one client
/// connection, and relay bytes bidirectionally.
///
/// Phase 1 — no real protocol, just raw byte relay.
/// Socket path convention is an open decision; see QUESTIONS.md Q1.
pub fn run(socket_path: &str) -> Result<(), ServerError> {
    let mut pty = PtyHandle::spawn(80, 24)?;
    let pty_reader = pty.take_reader();
    let pty_writer = pty.take_writer();

    // Remove a stale socket from a previous run so bind succeeds.
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    eprintln!("[server] listening on {socket_path}");

    let (stream, _) = listener.accept()?;
    let stream_writer = stream.try_clone()?;
    let mut stream_reader = stream;

    // Thread 1: PTY output → socket + state detection side-channel.
    let pty_to_sock = std::thread::spawn(move || {
        let mut reader = pty_reader;
        let mut writer = stream_writer;
        let mut detector = NaiveDetector::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Some(new_state) = detector.feed(&buf[..n]) {
                        eprintln!("[agentd] state → {new_state:?}");
                    }
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Thread 2: socket input → PTY stdin.
    let sock_to_pty = std::thread::spawn(move || {
        let mut writer = pty_writer;
        let mut buf = [0u8; 4096];
        loop {
            match stream_reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });

    pty_to_sock.join().ok();
    sock_to_pty.join().ok();

    let _ = pty.child.wait();
    Ok(())
}
