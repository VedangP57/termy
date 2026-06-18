use pty::PtyHandle;
use std::io::{self, Read, Write};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut pty = PtyHandle::spawn(80, 24)?;
    let mut pty_reader = pty.take_reader();
    let pty_writer = pty.take_writer();

    // Thread 1: PTY output → stdout
    let to_stdout = std::thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = stdout.flush();
                }
            }
        }
    });

    // Thread 2: stdin → PTY
    let from_stdin = std::thread::spawn(move || {
        let mut writer = pty_writer;
        let mut stdin = io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });

    to_stdout.join().ok();
    from_stdin.join().ok();

    let _ = pty.child.wait();
    Ok(())
}
