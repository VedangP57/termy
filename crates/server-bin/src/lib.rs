// Phase 8 — session-persistent server with wire protocol and agent JSON API.
//
// The server manages one pane (PTY + grid state). Clients connect, receive a
// GridReplay of buffered output for state reconstruction, then stream new
// PtyData. Disconnecting a client does NOT kill the pane — reattaching later
// replays the accumulated buffer.
//
// Transport is abstracted over `Box<dyn Read + Write + Send>` so the same code
// works over a Unix socket (local attach) and over stdin/stdout (SSH remote
// attach via `termd --server --attach-stdio`).
//
// A separate agent JSON socket at `<socket_path>-agent` exposes the pane
// state to external scripts without the binary wire protocol.

use agentd::{GridDetector, PaneState};
use protocol::{AgentState, ClientMsg, PaneInfo, ServerMsg, read_msg, write_msg};
use pty::PtyHandle;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use thiserror::Error;

const OUTPUT_BUF_MAX: usize = 512 * 1024; // 512 KB replay buffer

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("PTY error: {0}")]
    Pty(#[from] pty::PtyError),
    #[error("socket error: {0}")]
    Socket(#[from] std::io::Error),
}

// ── Shared pane state ────────────────────────────────────────────────────────

struct PaneShared {
    /// Ring buffer of all PTY output bytes — sent to new clients as GridReplay.
    output_buf: VecDeque<u8>,
    /// Agent-state detector.
    detector: GridDetector,
    /// Current pane dimensions.
    rows: u16,
    cols: u16,
    /// Current classified state (for agent API).
    state: AgentState,
    /// Writer to the PTY (input from client).
    pty_writer: Box<dyn Write + Send>,
    // Subscriber senders — subscribers get state-change notifications.
    // (Phase 8 basic: just log; full subscribe deferred per QUESTIONS.md)
}

impl PaneShared {
    fn feed_output(&mut self, bytes: &[u8]) -> Option<AgentState> {
        // Buffer for replay.
        for &b in bytes {
            self.output_buf.push_back(b);
        }
        while self.output_buf.len() > OUTPUT_BUF_MAX {
            self.output_buf.pop_front();
        }
        // Run state detection.
        if let Some(new_ps) = self.detector.feed(bytes) {
            let new_as = pane_to_agent_state(&new_ps);
            self.state = new_as.clone();
            Some(new_as)
        } else {
            None
        }
    }

    fn replay_bytes(&self) -> Vec<u8> {
        self.output_buf.iter().copied().collect()
    }

    fn send_input(&mut self, data: &[u8]) -> io::Result<()> {
        self.pty_writer.write_all(data)
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        self.detector = GridDetector::new(cols, rows);
    }

    fn recent_lines(&self, n: usize) -> Vec<String> {
        // Convert the replay buffer to UTF-8 text and return the last n lines.
        let raw_bytes = self.replay_bytes();
        let text = String::from_utf8_lossy(&raw_bytes);
        // Strip ANSI codes with a simple pass (only printable ASCII lines).
        let stripped = strip_ansi(text.as_ref());
        let all: Vec<&str> = stripped.lines().collect();
        let start = all.len().saturating_sub(n);
        all[start..].iter().map(|l| l.to_string()).collect()
    }
}

fn pane_to_agent_state(ps: &PaneState) -> AgentState {
    match ps {
        PaneState::Idle    => AgentState::Idle,
        PaneState::Working => AgentState::Working,
        PaneState::Blocked => AgentState::Blocked,
        PaneState::Done    => AgentState::Done,
    }
}

/// Very minimal ANSI stripper: removes ESC[…m and similar CSI sequences.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Consume until a letter (final byte).
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() { break; }
                }
            } else {
                // ESC followed by a single char — skip both.
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out
}

// ── Main server entry point ──────────────────────────────────────────────────

/// Run the server.
///
/// If `stdio_mode` is true, the server reads its transport from stdin/stdout
/// (for SSH remote-attach: `ssh host termd --server --attach-stdio`).
/// Otherwise it listens on `socket_path` (Unix domain socket).
pub fn run(socket_path: &str, stdio_mode: bool) -> Result<(), ServerError> {
    let mut pty = PtyHandle::spawn(80, 24)?;
    let pty_reader = pty.take_reader();
    let pty_writer = pty.take_writer();

    let pane = Arc::new(Mutex::new(PaneShared {
        output_buf: VecDeque::new(),
        detector:   GridDetector::new(80, 24),
        rows:       24,
        cols:       80,
        state:      AgentState::Idle,
        pty_writer,
    }));

    // Thread: PTY output → update shared state, broadcast to active client.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let pane_pty = Arc::clone(&pane);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut reader = pty_reader;
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let bytes = buf[..n].to_vec();
                    {
                        let mut p = pane_pty.lock().unwrap();
                        if let Some(state) = p.feed_output(&bytes) {
                            eprintln!("[agentd] state → {state:?}");
                        }
                    }
                    // Forward to active client (ignore if no client attached).
                    let _ = tx.send(bytes);
                }
            }
        }
    });

    // Thread: fan-out PTY bytes to active client socket writer.
    // We use a channel of writers — the current writer is swapped on each attach/detach.
    let (client_tx, client_rx) = std::sync::mpsc::sync_channel::<Option<Box<dyn Write + Send>>>(1);
    std::thread::spawn(move || {
        let mut current: Option<Box<dyn Write + Send>> = None;
        loop {
            // Check for a new client or disconnect.
            if let Ok(opt) = client_rx.try_recv() {
                current = opt;
            }
            // Forward PTY bytes to current client.
            if let Ok(bytes) = rx.recv_timeout(std::time::Duration::from_millis(10)) {
                if let Some(ref mut w) = current {
                    let msg = ServerMsg::PtyData { data: bytes };
                    if write_msg(w, &msg).is_err() {
                        current = None;
                    }
                }
                // Re-check client channel after each PTY chunk.
                if let Ok(opt) = client_rx.try_recv() {
                    current = opt;
                }
            }
        }
    });

    // Agent JSON API socket.
    let agent_socket_path = format!("{socket_path}-agent");
    let pane_agent = Arc::clone(&pane);
    let agent_path_clone = agent_socket_path.clone();
    std::thread::spawn(move || {
        let _ = std::fs::remove_file(&agent_path_clone);
        let listener = match UnixListener::bind(&agent_path_clone) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[agent-api] bind failed: {e}");
                return;
            }
        };
        // Set permissions so only the owning user can access it.
        let _ = std::fs::set_permissions(
            &agent_path_clone,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        );
        eprintln!("[agent-api] listening on {agent_path_clone}");
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let pane_clone = Arc::clone(&pane_agent);
                    std::thread::spawn(move || {
                        handle_agent_client(s, pane_clone);
                    });
                }
                Err(_) => break,
            }
        }
    });

    if stdio_mode {
        // SSH remote-attach: speak the protocol over stdin/stdout.
        // Use io::stdin()/stdout() directly — StdinLock is not Send.
        eprintln!("[server] stdio mode — ready for remote attach");
        handle_client_transport(
            Box::new(io::stdin()),
            Box::new(io::stdout()),
            &pane,
            &client_tx,
        );
    } else {
        // Local Unix socket: accept clients in a loop (pane persists).
        let _ = std::fs::remove_file(socket_path);
        let listener = UnixListener::bind(socket_path)?;
        // Set socket permissions to owner-only.
        let _ = std::fs::set_permissions(
            socket_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        );
        eprintln!("[server] listening on {socket_path}");
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let reader = match s.try_clone() {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    handle_client_transport(
                        Box::new(reader),
                        Box::new(s),
                        &pane,
                        &client_tx,
                    );
                }
                Err(e) => {
                    eprintln!("[server] accept error: {e}");
                }
            }
        }
    }

    Ok(())
}

fn handle_client_transport(
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
    pane: &Arc<Mutex<PaneShared>>,
    client_tx: &std::sync::mpsc::SyncSender<Option<Box<dyn Write + Send>>>,
) {
    // Read the first message — expect Attach.
    let msg: ClientMsg = match read_msg(&mut reader) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[server] handshake read error: {e}");
            return;
        }
    };
    match msg {
        ClientMsg::Attach { .. } => {}
        ClientMsg::ListPanes => {
            let p = pane.lock().unwrap();
            let info = PaneInfo { id: 0, state: p.state.clone(), rows: p.rows, cols: p.cols };
            let _ = write_msg(&mut writer, &ServerMsg::PaneList { panes: vec![info] });
            return;
        }
        _ => {
            eprintln!("[server] expected Attach as first message");
            return;
        }
    }

    // Send GridReplay so the client can reconstruct pane state.
    {
        let p = pane.lock().unwrap();
        let replay = p.replay_bytes();
        if let Err(e) = write_msg(&mut writer, &ServerMsg::GridReplay { data: replay }) {
            eprintln!("[server] GridReplay send error: {e}");
            return;
        }
    }

    // Register this writer as the active streaming target.
    let _ = client_tx.send(Some(writer));

    // Read loop: client → PTY.
    loop {
        let msg: ClientMsg = match read_msg(&mut reader) {
            Ok(m) => m,
            Err(_) => {
                // Client disconnected — keep pane alive, signal no active client.
                eprintln!("[server] client disconnected (pane kept alive)");
                let _ = client_tx.send(None);
                break;
            }
        };
        match msg {
            ClientMsg::Input { data } => {
                let mut p = pane.lock().unwrap();
                let _ = p.send_input(&data);
            }
            ClientMsg::Resize { rows, cols } => {
                let mut p = pane.lock().unwrap();
                p.resize(rows, cols);
            }
            ClientMsg::Detach => {
                eprintln!("[server] client detached (pane kept alive)");
                let _ = client_tx.send(None);
                break;
            }
            ClientMsg::ListPanes => {}
            ClientMsg::ReadOutput { .. } => {}
            ClientMsg::Attach { .. } => {}
        }
    }
}

// ── Agent JSON API ───────────────────────────────────────────────────────────

fn handle_agent_client(stream: UnixStream, pane: Arc<Mutex<PaneShared>>) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let response = handle_agent_command(line.trim(), &pane);
        let _ = writeln!(writer, "{response}");
    }
}

fn handle_agent_command(cmd: &str, pane: &Arc<Mutex<PaneShared>>) -> String {
    // Minimal JSON parsing — match on known command patterns without a dep.
    if cmd.contains("\"list_panes\"") {
        let p = pane.lock().unwrap();
        return format!(
            r#"{{"panes":[{{"id":0,"state":"{}","rows":{},"cols":{}}}]}}"#,
            p.state, p.rows, p.cols
        );
    }
    if cmd.contains("\"read_output\"") {
        let lines_n = extract_json_u64(cmd, "lines").unwrap_or(50) as usize;
        let p = pane.lock().unwrap();
        let lines = p.recent_lines(lines_n);
        let escaped: Vec<String> = lines.iter()
            .map(|l| format!("\"{}\"", l.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        return format!(r#"{{"lines":[{}]}}"#, escaped.join(","));
    }
    if cmd.contains("\"send_input\"") {
        if let Some(data_str) = extract_json_str(cmd, "data") {
            let mut p = pane.lock().unwrap();
            let _ = p.send_input(data_str.as_bytes());
            return r#"{"ok":true}"#.to_string();
        }
    }
    r#"{"error":"unknown command"}"#.to_string()
}

fn extract_json_u64(json: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{}\":", key);
    let pos = json.find(&pattern)?;
    let rest = json[pos + pattern.len()..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_json_str(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":\"", key);
    let pos = json.find(&pattern)?;
    let rest = &json[pos + pattern.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

