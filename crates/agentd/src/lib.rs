use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentdError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The four states a pane can be in, as defined in
/// `docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 7.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneState {
    /// Shell prompt is showing; nothing is running.
    Idle,
    /// A foreground process is running and has produced output recently.
    Working,
    /// A foreground process appears to be waiting on user input.
    Blocked,
    /// A foreground process exited but the pane has not been acknowledged.
    Done,
}

/// Phase 1 naive detector — operates on the raw PTY byte stream.
///
/// This is intentionally simple and will misclassify in many real cases.
/// Phase 2 rewrites this to operate on the parsed grid/line model from
/// `crates/vt` rather than raw bytes.
pub struct NaiveDetector {
    /// `None` until the first byte is fed; guarantees the first feed always emits a state.
    state: Option<PaneState>,
    /// Rolling window of recent bytes for heuristic matching.
    buf: Vec<u8>,
}

impl Default for NaiveDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl NaiveDetector {
    pub fn new() -> Self {
        Self {
            state: None,
            buf: Vec::with_capacity(512),
        }
    }

    /// Feed bytes from the PTY output stream.
    ///
    /// Returns the new `PaneState` if it changed (or if this is the first feed), or `None` if unchanged.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<PaneState> {
        self.buf.extend_from_slice(bytes);
        // Bound memory: keep only the last 512 bytes.
        if self.buf.len() > 512 {
            let excess = self.buf.len() - 512;
            self.buf.drain(..excess);
        }
        let new_state = self.classify();
        if self.state.as_ref() != Some(&new_state) {
            self.state = Some(new_state.clone());
            Some(new_state)
        } else {
            None
        }
    }

    pub fn state(&self) -> Option<&PaneState> {
        self.state.as_ref()
    }

    /// Classify current buffer contents as Idle or Working.
    ///
    /// Heuristic: strip ANSI escapes, then look for a common shell prompt
    /// suffix on the last non-empty line. Works for default zsh/bash prompts.
    /// Blocked and Done are not detectable from raw bytes; those come in Phase 2.
    fn classify(&self) -> PaneState {
        let text = String::from_utf8_lossy(&self.buf);
        let stripped = strip_ansi_escapes(&text);
        let last = stripped
            .lines()
            .filter(|l| !l.trim().is_empty())
            .next_back()
            .unwrap_or("");

        if last.ends_with("$ ")
            || last.ends_with("% ")
            || last.ends_with("# ")
            || last.ends_with("> ")
        {
            PaneState::Idle
        } else {
            PaneState::Working
        }
    }
}

/// Remove ANSI CSI escape sequences so prompt-suffix matching is not confused
/// by color codes. Phase 1 naive implementation — sufficient for the stub.
fn strip_ansi_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            // Consume parameter/intermediate bytes until the final byte (a letter).
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_on_zsh_prompt() {
        let mut d = NaiveDetector::new();
        let result = d.feed(b"some output\n~/termy % ");
        assert_eq!(result, Some(PaneState::Idle));
    }

    #[test]
    fn working_on_mid_command_output() {
        let mut d = NaiveDetector::new();
        d.feed(b"~/termy % "); // prime to Idle
        let result = d.feed(b"running something\nmore output here");
        assert_eq!(result, Some(PaneState::Working));
    }

    #[test]
    fn idle_on_bash_prompt() {
        let mut d = NaiveDetector::new();
        let result = d.feed(b"user@host:~$ ");
        assert_eq!(result, Some(PaneState::Idle));
    }

    #[test]
    fn no_change_returns_none() {
        let mut d = NaiveDetector::new();
        d.feed(b"line one\nline two"); // starts Working
        let result = d.feed(b"line three\nline four"); // still Working
        assert_eq!(result, None);
    }

    #[test]
    fn ansi_codes_dont_confuse_idle_detection() {
        let mut d = NaiveDetector::new();
        // Prompt wrapped in ANSI color codes
        let result = d.feed(b"\x1b[32muser@host\x1b[0m:~$ ");
        assert_eq!(result, Some(PaneState::Idle));
    }
}
