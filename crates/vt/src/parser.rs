/// Actions emitted by the VT parser state machine. The grid/screen
/// processes these one at a time rather than dealing with raw bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Printable Unicode character to place at the cursor.
    Print(char),
    /// C0/C1 control character (e.g. 0x08 BS, 0x09 HT, 0x0A LF, 0x0D CR, 0x07 BEL).
    Execute(u8),
    /// A completed CSI sequence.
    CsiDispatch {
        params: Vec<u32>,
        /// If the first param byte was 0x3C-0x3F, private[0] holds that byte.
        private_marker: Option<u8>,
        intermediates: Vec<u8>,
        final_byte: u8,
    },
    /// A completed OSC sequence. params[0] is the numeric command.
    OscDispatch(Vec<Vec<u8>>),
    /// ESC followed by a final byte (not '[' or ']').
    EscDispatch {
        intermediates: Vec<u8>,
        byte: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    OscString,
    OscStringEscape,
    // DCS / SOS / PM / APC are silently ignored for now.
    Ignore,
}

/// Streaming VT500-compatible parser.
///
/// Feed byte slices via `advance`; collect `Action`s via the callback.
/// Implements the Paul Williams DEC VT500 state machine.
pub struct Parser {
    state: State,
    params: Vec<u32>,
    current_param: u32,
    private_marker: Option<u8>,
    intermediates: Vec<u8>,
    osc_buf: Vec<u8>,
    /// UTF-8 reassembly buffer (up to 4 bytes).
    utf8_buf: [u8; 4],
    utf8_len: usize,
    utf8_needed: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            params: Vec::with_capacity(16),
            current_param: 0,
            private_marker: None,
            intermediates: Vec::with_capacity(4),
            osc_buf: Vec::with_capacity(64),
            utf8_buf: [0; 4],
            utf8_len: 0,
            utf8_needed: 0,
        }
    }

    /// Feed bytes into the parser; calls `cb` for each completed Action.
    pub fn advance(&mut self, bytes: &[u8], cb: &mut dyn FnMut(Action)) {
        for &b in bytes {
            self.feed(b, cb);
        }
    }

    fn feed(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        // UTF-8 multi-byte reassembly — only active in Ground state.
        if self.state == State::Ground && self.utf8_needed > 0 {
            if b & 0xC0 == 0x80 {
                self.utf8_buf[self.utf8_len] = b;
                self.utf8_len += 1;
                if self.utf8_len == self.utf8_needed {
                    let s = &self.utf8_buf[..self.utf8_len];
                    if let Ok(s) = std::str::from_utf8(s) {
                        if let Some(ch) = s.chars().next() {
                            cb(Action::Print(ch));
                        }
                    }
                    self.utf8_len = 0;
                    self.utf8_needed = 0;
                }
                return;
            } else {
                // Invalid continuation — drop sequence, fall through to process b normally.
                self.utf8_len = 0;
                self.utf8_needed = 0;
            }
        }

        match self.state {
            State::Ground => self.ground(b, cb),
            State::Escape => self.escape(b, cb),
            State::EscIntermediate => self.esc_intermediate(b, cb),
            State::CsiEntry => self.csi_entry(b, cb),
            State::CsiParam => self.csi_param(b, cb),
            State::CsiIntermediate => self.csi_intermediate(b, cb),
            State::CsiIgnore => self.csi_ignore(b, cb),
            State::OscString => self.osc_string(b, cb),
            State::OscStringEscape => self.osc_string_escape(b, cb),
            State::Ignore => {
                // Silently consume until ST or BEL.
                if b == 0x07 || b == 0x9C {
                    self.state = State::Ground;
                }
                if b == 0x1B {
                    self.state = State::Ignore; // wait for '\\'
                }
            }
        }
    }

    fn ground(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        match b {
            0x1B => {
                self.state = State::Escape;
            }
            0x00..=0x1A | 0x1C..=0x1F => {
                // C0 controls (excluding ESC=0x1B).
                cb(Action::Execute(b));
            }
            0x20..=0x7E => {
                cb(Action::Print(b as char));
            }
            0x7F => {
                // DEL — treated as Execute in most terminals.
                cb(Action::Execute(b));
            }
            // UTF-8 multi-byte start bytes.
            0xC0..=0xDF => {
                self.utf8_buf[0] = b;
                self.utf8_len = 1;
                self.utf8_needed = 2;
            }
            0xE0..=0xEF => {
                self.utf8_buf[0] = b;
                self.utf8_len = 1;
                self.utf8_needed = 3;
            }
            0xF0..=0xF7 => {
                self.utf8_buf[0] = b;
                self.utf8_len = 1;
                self.utf8_needed = 4;
            }
            _ => {} // 0x80-0xBF stray continuations, 0xF8+ illegal — skip.
        }
    }

    fn escape(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        self.params.clear();
        self.current_param = 0;
        self.private_marker = None;
        self.intermediates.clear();

        match b {
            0x5B => {
                // '['  → CSI
                self.state = State::CsiEntry;
            }
            0x5D => {
                // ']'  → OSC
                self.osc_buf.clear();
                self.state = State::OscString;
            }
            0x50 | 0x58 | 0x5E | 0x5F => {
                // DCS, SOS, PM, APC — ignore until ST.
                self.state = State::Ignore;
            }
            0x1B => {
                // ESC ESC — stay in Escape for the next byte.
            }
            0x20..=0x2F => {
                self.intermediates.push(b);
                self.state = State::EscIntermediate;
            }
            0x30..=0x7E => {
                cb(Action::EscDispatch {
                    intermediates: self.intermediates.clone(),
                    byte: b,
                });
                self.state = State::Ground;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn esc_intermediate(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        match b {
            0x20..=0x2F => {
                self.intermediates.push(b);
            }
            0x30..=0x7E => {
                cb(Action::EscDispatch {
                    intermediates: self.intermediates.clone(),
                    byte: b,
                });
                self.state = State::Ground;
            }
            0x1B => {
                self.state = State::Escape;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn csi_entry(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        match b {
            0x30..=0x39 => {
                // digit
                self.current_param = b as u32 - 0x30;
                self.state = State::CsiParam;
            }
            0x3B => {
                // ';' separator with implicit leading zero
                self.params.push(0);
                self.state = State::CsiParam;
            }
            0x3C..=0x3F => {
                // private marker (<, =, >, ?)
                self.private_marker = Some(b);
                self.state = State::CsiParam;
            }
            0x20..=0x2F => {
                self.intermediates.push(b);
                self.state = State::CsiIntermediate;
            }
            0x40..=0x7E => {
                self.dispatch_csi(b, cb);
            }
            0x1B => {
                self.state = State::Escape;
            }
            _ => {}
        }
    }

    fn csi_param(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        match b {
            0x30..=0x39 => {
                self.current_param = self.current_param.saturating_mul(10).saturating_add(b as u32 - 0x30);
            }
            0x3B => {
                self.params.push(self.current_param);
                self.current_param = 0;
            }
            0x3C..=0x3F => {
                // Private modifier after params start → ignore sequence.
                self.state = State::CsiIgnore;
            }
            0x20..=0x2F => {
                self.params.push(self.current_param);
                self.current_param = 0;
                self.intermediates.push(b);
                self.state = State::CsiIntermediate;
            }
            0x40..=0x7E => {
                self.params.push(self.current_param);
                self.current_param = 0;
                self.dispatch_csi(b, cb);
            }
            0x1B => {
                self.state = State::Escape;
            }
            _ => {}
        }
    }

    fn csi_intermediate(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        match b {
            0x20..=0x2F => {
                self.intermediates.push(b);
            }
            0x40..=0x7E => {
                self.dispatch_csi(b, cb);
            }
            0x30..=0x3F => {
                self.state = State::CsiIgnore;
            }
            0x1B => {
                self.state = State::Escape;
            }
            _ => {}
        }
    }

    fn csi_ignore(&mut self, b: u8, _cb: &mut dyn FnMut(Action)) {
        match b {
            0x40..=0x7E => {
                self.state = State::Ground;
            }
            0x1B => {
                self.state = State::Escape;
            }
            _ => {}
        }
    }

    fn dispatch_csi(&mut self, final_byte: u8, cb: &mut dyn FnMut(Action)) {
        cb(Action::CsiDispatch {
            params: self.params.clone(),
            private_marker: self.private_marker,
            intermediates: self.intermediates.clone(),
            final_byte,
        });
        self.state = State::Ground;
    }

    fn osc_string(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        match b {
            0x07 => {
                // BEL terminates OSC.
                self.dispatch_osc(cb);
            }
            0x1B => {
                self.state = State::OscStringEscape;
            }
            0x00..=0x06 | 0x08..=0x1A | 0x1C..=0x1F => {
                // C0 other than BEL/ESC — skip.
            }
            _ => {
                self.osc_buf.push(b);
            }
        }
    }

    fn osc_string_escape(&mut self, b: u8, cb: &mut dyn FnMut(Action)) {
        if b == 0x5C {
            // ESC \ = ST
            self.dispatch_osc(cb);
        } else {
            // Not a valid ST — return to OSC collection.
            self.state = State::OscString;
        }
    }

    fn dispatch_osc(&mut self, cb: &mut dyn FnMut(Action)) {
        // Split on first ';': params[0] = command number, params[1] = data.
        let parts: Vec<Vec<u8>> = self.osc_buf.splitn(2, |&b| b == b';').map(|s| s.to_vec()).collect();
        cb(Action::OscDispatch(parts));
        self.osc_buf.clear();
        self.state = State::Ground;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Vec<Action> {
        let mut p = Parser::new();
        let mut out = Vec::new();
        p.advance(input, &mut |a| out.push(a));
        out
    }

    #[test]
    fn plain_ascii() {
        let actions = parse(b"hi");
        assert_eq!(actions, vec![Action::Print('h'), Action::Print('i')]);
    }

    #[test]
    fn csi_cursor_up() {
        let actions = parse(b"\x1b[2A");
        assert_eq!(actions, vec![Action::CsiDispatch {
            params: vec![2],
            private_marker: None,
            intermediates: vec![],
            final_byte: b'A',
        }]);
    }

    #[test]
    fn csi_no_params_defaults_empty() {
        let actions = parse(b"\x1b[H");
        assert_eq!(actions, vec![Action::CsiDispatch {
            params: vec![],
            private_marker: None,
            intermediates: vec![],
            final_byte: b'H',
        }]);
    }

    #[test]
    fn csi_sgr_reset() {
        let actions = parse(b"\x1b[0m");
        assert_eq!(actions, vec![Action::CsiDispatch {
            params: vec![0],
            private_marker: None,
            intermediates: vec![],
            final_byte: b'm',
        }]);
    }

    #[test]
    fn csi_private_mode() {
        // ESC[?1049h — enter alt screen
        let actions = parse(b"\x1b[?1049h");
        assert_eq!(actions, vec![Action::CsiDispatch {
            params: vec![1049],
            private_marker: Some(b'?'),
            intermediates: vec![],
            final_byte: b'h',
        }]);
    }

    #[test]
    fn execute_cr_lf() {
        let actions = parse(b"\r\n");
        assert_eq!(actions, vec![Action::Execute(0x0D), Action::Execute(0x0A)]);
    }

    #[test]
    fn utf8_two_byte() {
        // '©' = U+00A9 = 0xC2 0xA9
        let actions = parse(&[0xC2, 0xA9]);
        assert_eq!(actions, vec![Action::Print('©')]);
    }

    #[test]
    fn utf8_three_byte() {
        // '→' = U+2192 = 0xE2 0x86 0x92
        let actions = parse(&[0xE2, 0x86, 0x92]);
        assert_eq!(actions, vec![Action::Print('→')]);
    }

    #[test]
    fn osc_title() {
        // OSC 2 ; title BEL
        let actions = parse(b"\x1b]2;my title\x07");
        assert_eq!(actions, vec![Action::OscDispatch(vec![b"2".to_vec(), b"my title".to_vec()])]);
    }
}
