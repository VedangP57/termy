// Phase 8 — wire protocol types shared by client and server.
// Framing: 4-byte little-endian length prefix, followed by bincode payload.
// PTY data is sent as ServerMsg::PtyData (Vec<u8> payload inside the frame).
// Damage computation is client-side — see QUESTIONS.md Q4.

use std::io::{self, Read, Write};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Ser(#[from] Box<bincode::ErrorKind>),
}

// ── Wire types ────────────────────────────────────────────────────────────────

/// Agent/pane state exposed over the socket API.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Done,
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle    => write!(f, "Idle"),
            AgentState::Working => write!(f, "Working"),
            AgentState::Blocked => write!(f, "Blocked"),
            AgentState::Done    => write!(f, "Done"),
        }
    }
}

/// Summary of a managed pane.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PaneInfo {
    pub id:    u32,
    pub state: AgentState,
    pub rows:  u16,
    pub cols:  u16,
}

/// Messages sent by the client to the server.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientMsg {
    /// Attach to pane `pane_id`, requesting a GridReplay first.
    Attach { pane_id: u32 },
    /// Detach from the current pane (server keeps it alive).
    Detach,
    /// Send bytes to the pane's PTY (keyboard input, etc.).
    Input { data: Vec<u8> },
    /// Notify server that the client window was resized.
    Resize { rows: u16, cols: u16 },
    /// Query the list of active panes (used by agent API clients too).
    ListPanes,
    /// Read the last `lines` lines of the pane's scrollback text.
    ReadOutput { lines: usize },
}

/// Messages sent by the server to the client.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMsg {
    /// Response to ListPanes.
    PaneList { panes: Vec<PaneInfo> },
    /// Sent immediately after Attach: all PTY bytes emitted since the pane
    /// was created (capped at ~512 KB), so the client can reconstruct state
    /// by feeding them to a fresh Terminal.
    GridReplay { data: Vec<u8> },
    /// Streaming PTY output bytes after the initial replay.
    PtyData { data: Vec<u8> },
    /// The agent-state detector changed state.
    StateChange { pane_id: u32, state: AgentState },
    /// Response to ReadOutput: recent output lines as strings.
    OutputLines { lines: Vec<String> },
}

// ── Framing ───────────────────────────────────────────────────────────────────

/// Write one length-prefixed bincode frame to `w`.
pub fn write_msg<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let encoded = bincode::serialize(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = encoded.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&encoded)?;
    w.flush()
}

/// Read one length-prefixed bincode frame from `r`.
pub fn read_msg<R: Read, T: serde::de::DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    // Cap at 4 MiB to avoid OOM on malformed frames.
    if len > 4 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
