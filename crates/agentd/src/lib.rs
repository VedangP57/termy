use thiserror::Error;
use vt::Terminal;

#[derive(Error, Debug)]
pub enum AgentdError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The four states a pane can be in.
/// See `docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 7.1.
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

/// Phase 2 detector — operates on the parsed grid from `crates/vt`.
///
/// This is more reliable than the Phase 1 raw-bytes heuristic because it
/// reads character content from the parsed grid rather than trying to
/// strip ANSI codes manually from a rolling byte window.
pub struct GridDetector {
    terminal: Terminal,
    state: Option<PaneState>,
}

impl GridDetector {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            terminal: Terminal::new(rows as usize, cols as usize),
            state: None,
        }
    }

    /// Feed raw PTY bytes. Returns the new `PaneState` if it changed
    /// (or if this is the first feed), or `None` if unchanged.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<PaneState> {
        self.terminal.advance(bytes);
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

    /// Access the underlying Terminal for debug dumps or further inspection.
    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    /// Classify the current grid contents as Idle or Working.
    ///
    /// Reads the last non-blank line from the parsed grid and checks for
    /// common shell prompt suffixes. Blocked/Done still require process-level
    /// information not available in Phase 2 — those remain future work.
    fn classify(&self) -> PaneState {
        let last = self.terminal.last_line_text();
        if is_shell_prompt(&last) {
            PaneState::Idle
        } else {
            PaneState::Working
        }
    }
}

/// Return true if the line looks like a shell prompt.
///
/// Checks for the conventional suffix characters used by zsh, bash, fish,
/// and other common shells. This intentionally matches a narrow set to avoid
/// false positives from command output that happens to end with '> '.
fn is_shell_prompt(line: &str) -> bool {
    // Common prompt endings: "$ ", "% ", "# ", "> "
    // Also accept them at the very end of the string without a trailing space
    // (some shells don't emit the trailing space until after the cursor).
    line.ends_with("$ ")
        || line.ends_with("% ")
        || line.ends_with("# ")
        || line.ends_with("> ")
        || line.trim_end().ends_with('$')
        || line.trim_end().ends_with('%')
        || line.trim_end().ends_with('#')
        || line.trim_end().ends_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(input: &[u8]) -> Option<PaneState> {
        let mut d = GridDetector::new(80, 24);
        d.feed(input)
    }

    #[test]
    fn idle_on_zsh_prompt() {
        assert_eq!(detect(b"some output\r\n~/termy % "), Some(PaneState::Idle));
    }

    #[test]
    fn idle_on_bash_prompt() {
        assert_eq!(detect(b"user@host:~$ "), Some(PaneState::Idle));
    }

    #[test]
    fn idle_with_ansi_color_codes() {
        // Shell prompt wrapped in SGR color codes — the grid strips these.
        assert_eq!(detect(b"\x1b[32muser@host\x1b[0m:~$ "), Some(PaneState::Idle));
    }

    #[test]
    fn working_on_mid_command_output() {
        let mut d = GridDetector::new(80, 24);
        d.feed(b"~/termy % "); // prime to Idle
        let result = d.feed(b"running something\r\nmore output here");
        assert_eq!(result, Some(PaneState::Working));
    }

    #[test]
    fn no_change_returns_none() {
        let mut d = GridDetector::new(80, 24);
        d.feed(b"line one\r\nline two"); // Working
        let result = d.feed(b"line three\r\nline four"); // still Working
        assert_eq!(result, None);
    }

    #[test]
    fn fish_prompt() {
        // fish shell uses '> ' as default prompt suffix
        assert_eq!(detect(b"~/termy> "), Some(PaneState::Idle));
    }
}
