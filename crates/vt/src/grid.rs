use std::collections::VecDeque;

use crate::cell::{Attrs, Cell};
use crate::color::{Color, ANSI_BRIGHT, ANSI_NAMED};
use crate::parser::Action;

const MAX_SCROLLBACK: usize = 10_000;

/// Mouse reporting mode set by the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    Off,
    X10,          // mode 1000 — button press/release only
    ButtonMotion, // mode 1002 — press/release + motion while held
    AnyMotion,    // mode 1003 — press/release + all motion
}

/// Cursor state.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub row:     usize,
    pub col:     usize,
    pub visible: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self { row: 0, col: 0, visible: true }
    }
}

/// A rectangular grid of cells.
#[derive(Debug, Clone)]
pub struct Grid {
    pub rows: usize,
    pub cols: usize,
    cells: Vec<Vec<Cell>>,
}

impl Grid {
    pub fn new(rows: usize, cols: usize) -> Self {
        let cells = vec![vec![Cell::blank(); cols]; rows];
        Self { rows, cols, cells }
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row][col]
    }

    pub fn cell_mut(&mut self, row: usize, col: usize) -> &mut Cell {
        &mut self.cells[row][col]
    }

    pub fn row(&self, row: usize) -> &[Cell] {
        &self.cells[row]
    }

    pub fn fill_row_range(&mut self, row: usize, start: usize, end: usize, attrs: &Attrs) {
        let end = end.min(self.cols);
        for col in start..end {
            self.cells[row][col] = Cell::blank_with(attrs.clone());
        }
    }

    /// Scroll up by `n` lines within `top..=bottom`. Returns the scrolled-out rows.
    pub fn scroll_up(&mut self, top: usize, bottom: usize, n: usize, attrs: &Attrs) -> Vec<Vec<Cell>> {
        let n = n.min(bottom - top + 1);
        let mut scrolled = Vec::with_capacity(n);
        for _ in 0..n {
            scrolled.push(self.cells[top].clone());
            self.cells[top..=bottom].rotate_left(1);
            self.cells[bottom] = vec![Cell::blank_with(attrs.clone()); self.cols];
        }
        scrolled
    }

    pub fn scroll_down(&mut self, top: usize, bottom: usize, n: usize, attrs: &Attrs) {
        let n = n.min(bottom - top + 1);
        for _ in 0..n {
            self.cells[top..=bottom].rotate_right(1);
            self.cells[top] = vec![Cell::blank_with(attrs.clone()); self.cols];
        }
    }

    pub fn erase_row_before(&mut self, row: usize, col: usize, attrs: &Attrs) {
        let end = (col + 1).min(self.cols);
        self.fill_row_range(row, 0, end, attrs);
    }

    pub fn erase_row_from(&mut self, row: usize, col: usize, attrs: &Attrs) {
        self.fill_row_range(row, col, self.cols, attrs);
    }

    pub fn erase_row(&mut self, row: usize, attrs: &Attrs) {
        self.fill_row_range(row, 0, self.cols, attrs);
    }

    pub fn insert_lines(&mut self, row: usize, bottom: usize, n: usize, attrs: &Attrs) {
        let n = n.min(bottom - row + 1);
        for _ in 0..n {
            self.cells[row..=bottom].rotate_right(1);
            self.cells[row] = vec![Cell::blank_with(attrs.clone()); self.cols];
        }
    }

    pub fn delete_lines(&mut self, row: usize, bottom: usize, n: usize, attrs: &Attrs) {
        let n = n.min(bottom - row + 1);
        for _ in 0..n {
            self.cells[row..=bottom].rotate_left(1);
            self.cells[bottom] = vec![Cell::blank_with(attrs.clone()); self.cols];
        }
    }

    pub fn insert_chars(&mut self, row: usize, col: usize, n: usize, attrs: &Attrs) {
        let n = n.min(self.cols - col);
        self.cells[row][col..].rotate_right(n);
        for c in col..col + n {
            self.cells[row][c] = Cell::blank_with(attrs.clone());
        }
    }

    pub fn delete_chars(&mut self, row: usize, col: usize, n: usize, attrs: &Attrs) {
        let n = n.min(self.cols - col);
        self.cells[row][col..].rotate_left(n);
        let blank_start = self.cols - n;
        for c in blank_start..self.cols {
            self.cells[row][c] = Cell::blank_with(attrs.clone());
        }
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        for row in self.cells.iter_mut() {
            row.resize(cols, Cell::blank());
        }
        if rows > self.rows {
            for _ in self.rows..rows {
                self.cells.push(vec![Cell::blank(); cols]);
            }
        } else {
            self.cells.truncate(rows);
        }
        self.rows = rows;
        self.cols = cols;
    }
}

pub struct Screen {
    normal:        Grid,
    alt:           Grid,
    in_alt:        bool,
    scrollback:    VecDeque<Vec<Cell>>,
    cursor:        Cursor,
    saved_cursor:  Option<Cursor>,
    pub attrs:     Attrs,
    scroll_top:    usize,
    scroll_bottom: usize,
    pending_wrap:  bool,

    // ── Phase 6: mode flags ──────────────────────────────────────────────────
    pub mouse_mode:      MouseMode,
    pub mouse_sgr:       bool,   // mode 1006: SGR-encoded mouse events
    pub bracketed_paste: bool,   // mode 2004
    pub app_cursor_keys: bool,   // mode 1 (DECCKM): SS3 vs CSI arrow sequences
    pub app_keypad:      bool,   // set by ESC = / cleared by ESC >
    pub window_title:    Option<String>, // from OSC 0 / OSC 2
    pub cursor_shape:    u32,    // DECSCUSR value (0/2=steady block, 1=blink block, etc.)

    // Last printed character — for REP (CSI b).
    last_char: char,

    // Queued terminal responses (DSR, DA, etc.) to be written back to PTY.
    pub pending_responses: Vec<Vec<u8>>,

    // ── Phase 7: advanced protocols ─────────────────────────────────────────
    // Synchronized output (DEC mode 2026): suppress rendering mid-update.
    pub sync_output: bool,
    // Kitty keyboard protocol stack (push/pop via CSI > u / CSI < u).
    pub keyboard_modes: Vec<u32>,
}

impl Screen {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            normal: Grid::new(rows, cols),
            alt:    Grid::new(rows, cols),
            in_alt: false,
            scrollback:    VecDeque::new(),
            cursor:        Cursor::default(),
            saved_cursor:  None,
            attrs:         Attrs::default(),
            scroll_top:    0,
            scroll_bottom: rows.saturating_sub(1),
            pending_wrap:  false,

            mouse_mode:      MouseMode::Off,
            mouse_sgr:       false,
            bracketed_paste: false,
            app_cursor_keys: false,
            app_keypad:      false,
            window_title:    None,
            cursor_shape:    0,
            last_char:       ' ',
            pending_responses: Vec::new(),
            sync_output:     false,
            keyboard_modes:  Vec::new(),
        }
    }

    pub fn rows(&self) -> usize         { self.active().rows }
    pub fn cols(&self) -> usize         { self.active().cols }
    pub fn cursor(&self) -> &Cursor     { &self.cursor }
    pub fn scrollback(&self) -> &VecDeque<Vec<Cell>> { &self.scrollback }
    pub fn is_in_alt(&self) -> bool     { self.in_alt }

    fn active(&self) -> &Grid {
        if self.in_alt { &self.alt } else { &self.normal }
    }

    fn active_mut(&mut self) -> &mut Grid {
        if self.in_alt { &mut self.alt } else { &mut self.normal }
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        self.active().cell(row, col)
    }

    pub fn row(&self, row: usize) -> &[Cell] {
        self.active().row(row)
    }

    /// Return the cell at the given display row when the view is scrolled back
    /// by `scroll_offset` lines. Returns None when the display row would be
    /// beyond the top of the available scrollback.
    pub fn display_cell(&self, display_row: usize, col: usize, scroll_offset: usize) -> Option<&Cell> {
        let sb = &self.scrollback;
        let virtual_row = display_row as isize - scroll_offset as isize;
        if virtual_row >= 0 {
            let r = virtual_row as usize;
            if r < self.rows() {
                return Some(self.active().cell(r, col));
            }
            return None;
        }
        // negative virtual_row → into scrollback
        // virtual_row == -1 → newest scrollback line (sb.len()-1)
        let sb_idx = sb.len().checked_sub(((-virtual_row) as usize))?;
        sb.get(sb_idx)?.get(col)
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.normal.resize(rows, cols);
        self.alt.resize(rows, cols);
        self.scroll_top    = 0;
        self.scroll_bottom = rows.saturating_sub(1);
        self.cursor.row    = self.cursor.row.min(rows.saturating_sub(1));
        self.cursor.col    = self.cursor.col.min(cols.saturating_sub(1));
    }

    pub fn process(&mut self, action: Action) {
        match action {
            Action::Print(ch) => self.print(ch),
            Action::Execute(b) => self.execute(b),
            Action::CsiDispatch { params, private_marker, intermediates, final_byte } => {
                self.csi(params, private_marker, intermediates, final_byte);
            }
            Action::EscDispatch { intermediates, byte } => {
                self.esc(intermediates, byte);
            }
            Action::ApcDispatch(payload) => {
                // Kitty inline-image protocol: payload starts with 'G'.
                // Parse cleanly so display isn't corrupted; rendering is a no-op.
                if payload.first() == Some(&b'G') {
                    // Acknowledged: Kitty image command received; no rendering yet.
                }
                // All other APC payloads: silently ignore.
            }
            Action::OscDispatch(parts) => {
                // OSC 0 / OSC 2: set window title.
                if let Some(cmd_bytes) = parts.first() {
                    let cmd = std::str::from_utf8(cmd_bytes).unwrap_or("");
                    if cmd == "0" || cmd == "2" {
                        if let Some(title_bytes) = parts.get(1) {
                            self.window_title = std::str::from_utf8(title_bytes)
                                .ok()
                                .map(|s| s.to_owned());
                        }
                    }
                    // OSC 8 (hyperlinks): silently ignore.
                }
            }
        }
    }

    fn print(&mut self, ch: char) {
        if self.pending_wrap {
            self.pending_wrap = false;
            self.do_lf();
            self.cursor.col = 0;
        }
        let row = self.cursor.row;
        let col = self.cursor.col;
        let attrs = self.attrs.clone();
        let cell  = self.active_mut().cell_mut(row, col);
        cell.ch    = ch;
        cell.attrs = attrs;
        self.last_char = ch;
        let cols = self.active().cols;
        if col + 1 >= cols {
            self.pending_wrap = true;
        } else {
            self.cursor.col += 1;
        }
    }

    fn execute(&mut self, b: u8) {
        match b {
            0x08 => {
                if self.cursor.col > 0 { self.cursor.col -= 1; }
            }
            0x09 => {
                let cols = self.active().cols;
                self.cursor.col = (((self.cursor.col / 8) + 1) * 8).min(cols - 1);
            }
            0x0A | 0x0B | 0x0C => { self.do_lf(); }
            0x0D => { self.cursor.col = 0; self.pending_wrap = false; }
            _ => {}
        }
    }

    fn do_lf(&mut self) {
        self.pending_wrap = false;
        if self.cursor.row == self.scroll_bottom {
            let top    = self.scroll_top;
            let bottom = self.scroll_bottom;
            let attrs  = self.attrs.clone();
            let in_alt = self.in_alt;
            let scrolled = self.active_mut().scroll_up(top, bottom, 1, &attrs);
            if !in_alt && top == 0 {
                for line in scrolled {
                    self.scrollback.push_back(line);
                    if self.scrollback.len() > MAX_SCROLLBACK {
                        self.scrollback.pop_front();
                    }
                }
            }
        } else if self.cursor.row + 1 < self.rows() {
            self.cursor.row += 1;
        }
    }

    fn esc(&mut self, intermediates: Vec<u8>, byte: u8) {
        match (intermediates.as_slice(), byte) {
            ([], b'7') => { self.saved_cursor = Some(self.cursor.clone()); }
            ([], b'8') => {
                if let Some(c) = self.saved_cursor.take() { self.cursor = c; }
            }
            ([], b'c') => {
                let rows = self.rows();
                let cols = self.cols();
                *self = Screen::new(rows, cols);
            }
            // DECKPAM / DECKPNM — keypad application / normal mode.
            ([], b'=') => { self.app_keypad = true; }
            ([], b'>') => { self.app_keypad = false; }
            _ => {}
        }
    }

    fn csi(&mut self, mut params: Vec<u32>, private_marker: Option<u8>, intermediates: Vec<u8>, final_byte: u8) {
        fn p(params: &[u32], idx: usize) -> u32 {
            params.get(idx).copied().unwrap_or(0)
        }
        fn p1(params: &[u32], idx: usize) -> u32 {
            let v = params.get(idx).copied().unwrap_or(0);
            if v == 0 { 1 } else { v }
        }

        // DECSCUSR (cursor shape): CSI Ps SP q — intermediate is 0x20 (space).
        if intermediates.first() == Some(&b' ') && final_byte == b'q' {
            self.cursor_shape = p(&params, 0);
            return;
        }

        match (private_marker, final_byte) {
            // ── Cursor movement ──────────────────────────────────────────────
            (None, b'A') => {
                let n = p1(&params, 0) as usize;
                self.cursor.row = self.cursor.row.saturating_sub(n).max(self.scroll_top);
                self.pending_wrap = false;
            }
            (None, b'B') => {
                let n = p1(&params, 0) as usize;
                self.cursor.row = (self.cursor.row + n).min(self.scroll_bottom);
                self.pending_wrap = false;
            }
            (None, b'C') => {
                let n = p1(&params, 0) as usize;
                let cols = self.cols();
                self.cursor.col = (self.cursor.col + n).min(cols - 1);
                self.pending_wrap = false;
            }
            (None, b'D') => {
                let n = p1(&params, 0) as usize;
                self.cursor.col = self.cursor.col.saturating_sub(n);
                self.pending_wrap = false;
            }
            (None, b'E') => {
                let n = p1(&params, 0) as usize;
                self.cursor.row = (self.cursor.row + n).min(self.scroll_bottom);
                self.cursor.col = 0;
                self.pending_wrap = false;
            }
            (None, b'F') => {
                let n = p1(&params, 0) as usize;
                self.cursor.row = self.cursor.row.saturating_sub(n).max(self.scroll_top);
                self.cursor.col = 0;
                self.pending_wrap = false;
            }
            (None, b'G') => {
                let cols = self.cols();
                self.cursor.col = (p1(&params, 0) as usize - 1).min(cols - 1);
                self.pending_wrap = false;
            }
            (None, b'H') | (None, b'f') => {
                let rows = self.rows();
                let cols = self.cols();
                self.cursor.row = (p1(&params, 0) as usize - 1).min(rows - 1);
                self.cursor.col = (p1(&params, 1) as usize - 1).min(cols - 1);
                self.pending_wrap = false;
            }
            (None, b'd') => {
                let rows = self.rows();
                self.cursor.row = (p1(&params, 0) as usize - 1).min(rows - 1);
                self.pending_wrap = false;
            }

            // ── Erase ────────────────────────────────────────────────────────
            (None, b'J') => {
                let attrs = self.attrs.clone();
                let rows  = self.rows();
                let row   = self.cursor.row;
                let col   = self.cursor.col;
                match p(&params, 0) {
                    0 => {
                        self.active_mut().erase_row_from(row, col, &attrs);
                        for r in row + 1..rows {
                            self.active_mut().erase_row(r, &attrs);
                        }
                    }
                    1 => {
                        for r in 0..row {
                            self.active_mut().erase_row(r, &attrs);
                        }
                        self.active_mut().erase_row_before(row, col, &attrs);
                    }
                    2 | 3 => {
                        for r in 0..rows {
                            self.active_mut().erase_row(r, &attrs);
                        }
                    }
                    _ => {}
                }
            }
            (None, b'K') => {
                let attrs = self.attrs.clone();
                let row   = self.cursor.row;
                let col   = self.cursor.col;
                match p(&params, 0) {
                    0 => self.active_mut().erase_row_from(row, col, &attrs),
                    1 => self.active_mut().erase_row_before(row, col, &attrs),
                    2 => self.active_mut().erase_row(row, &attrs),
                    _ => {}
                }
            }

            // ── Scroll ───────────────────────────────────────────────────────
            (None, b'S') => {
                let n      = p1(&params, 0) as usize;
                let top    = self.scroll_top;
                let bottom = self.scroll_bottom;
                let attrs  = self.attrs.clone();
                let in_alt = self.in_alt;
                let scrolled = self.active_mut().scroll_up(top, bottom, n, &attrs);
                if !in_alt && top == 0 {
                    for line in scrolled {
                        self.scrollback.push_back(line);
                        if self.scrollback.len() > MAX_SCROLLBACK {
                            self.scrollback.pop_front();
                        }
                    }
                }
            }
            (None, b'T') => {
                let n      = p1(&params, 0) as usize;
                let top    = self.scroll_top;
                let bottom = self.scroll_bottom;
                let attrs  = self.attrs.clone();
                self.active_mut().scroll_down(top, bottom, n, &attrs);
            }

            // ── Insert / delete ──────────────────────────────────────────────
            (None, b'L') => {
                let n      = p1(&params, 0) as usize;
                let bottom = self.scroll_bottom;
                let row    = self.cursor.row;
                let attrs  = self.attrs.clone();
                self.active_mut().insert_lines(row, bottom, n, &attrs);
            }
            (None, b'M') => {
                let n      = p1(&params, 0) as usize;
                let bottom = self.scroll_bottom;
                let row    = self.cursor.row;
                let attrs  = self.attrs.clone();
                self.active_mut().delete_lines(row, bottom, n, &attrs);
            }
            (None, b'@') => {
                let n           = p1(&params, 0) as usize;
                let (row, col)  = (self.cursor.row, self.cursor.col);
                let attrs       = self.attrs.clone();
                self.active_mut().insert_chars(row, col, n, &attrs);
            }
            (None, b'P') => {
                let n           = p1(&params, 0) as usize;
                let (row, col)  = (self.cursor.row, self.cursor.col);
                let attrs       = self.attrs.clone();
                self.active_mut().delete_chars(row, col, n, &attrs);
            }
            (None, b'X') => {
                let n           = p1(&params, 0) as usize;
                let (row, col)  = (self.cursor.row, self.cursor.col);
                let cols        = self.cols();
                let end         = (col + n).min(cols);
                let attrs       = self.attrs.clone();
                for c in col..end {
                    *self.active_mut().cell_mut(row, c) = Cell::blank_with(attrs.clone());
                }
            }

            // ── REP — repeat last printed character ──────────────────────────
            (None, b'b') => {
                let n  = p1(&params, 0) as usize;
                let ch = self.last_char;
                for _ in 0..n { self.print(ch); }
            }

            // ── Scroll region ────────────────────────────────────────────────
            (None, b'r') => {
                let rows   = self.rows();
                let top    = p1(&params, 0) as usize - 1;
                let bottom = if p(&params, 1) == 0 { rows - 1 } else { p(&params, 1) as usize - 1 };
                if top < bottom && bottom < rows {
                    self.scroll_top    = top;
                    self.scroll_bottom = bottom;
                }
                self.cursor.row = 0;
                self.cursor.col = 0;
                self.pending_wrap = false;
            }

            // ── Cursor save/restore ──────────────────────────────────────────
            (None, b's') => { self.saved_cursor = Some(self.cursor.clone()); }
            (None, b'u') => {
                if let Some(c) = self.saved_cursor.clone() { self.cursor = c; }
            }

            // ── SGR ──────────────────────────────────────────────────────────
            (None, b'm') => {
                if params.is_empty() { params.push(0); }
                self.apply_sgr(&params);
            }

            // ── Device status report ─────────────────────────────────────────
            (None, b'n') => {
                match p(&params, 0) {
                    5 => {
                        // DSR — device status: respond "terminal is ready"
                        self.pending_responses.push(b"\x1b[0n".to_vec());
                    }
                    6 => {
                        // CPR — cursor position report: respond with ESC[row;colR
                        let resp = format!("\x1b[{};{}R", self.cursor.row + 1, self.cursor.col + 1);
                        self.pending_responses.push(resp.into_bytes());
                    }
                    _ => {}
                }
            }

            // ── Primary device attributes ────────────────────────────────────
            (None, b'c') => {
                if p(&params, 0) == 0 {
                    // Identify as VT100 with no options.
                    self.pending_responses.push(b"\x1b[?1;0c".to_vec());
                }
            }

            // ── Private modes ────────────────────────────────────────────────
            (Some(b'?'), b'h') => {
                for mode in params.clone() { self.set_private_mode(mode, true); }
            }
            (Some(b'?'), b'l') => {
                for mode in params.clone() { self.set_private_mode(mode, false); }
            }

            // ── Kitty keyboard protocol ──────────────────────────────────────
            // CSI > flags u — push mode onto stack.
            (Some(b'>'), b'u') => {
                let flags = params.first().copied().unwrap_or(0);
                self.keyboard_modes.push(flags);
            }
            // CSI < n u — pop n entries from the stack.
            (Some(b'<'), b'u') => {
                let n = params.first().copied().unwrap_or(1) as usize;
                let new_len = self.keyboard_modes.len().saturating_sub(n);
                self.keyboard_modes.truncate(new_len);
            }
            // CSI ? u — query current keyboard flags; respond with CSI ? flags u.
            (Some(b'?'), b'u') => {
                let flags = self.keyboard_modes.last().copied().unwrap_or(0);
                let resp = format!("\x1b[?{}u", flags);
                self.pending_responses.push(resp.into_bytes());
            }

            _ => {}
        }
    }

    fn set_private_mode(&mut self, mode: u32, set: bool) {
        match mode {
            // DECCKM — application cursor keys.
            1 => { self.app_cursor_keys = set; }

            // DECAWM — auto-wrap mode. We always wrap; track the flag but don't change behaviour.
            7 => {}

            // Cursor blink — no visual difference yet; no-op.
            12 => {}

            // Cursor visibility.
            25 => { self.cursor.visible = set; }

            // Mouse button reporting (X10-compatible).
            1000 => {
                self.mouse_mode = if set { MouseMode::X10 } else { MouseMode::Off };
            }
            // Mouse button + motion while pressed.
            1002 => {
                self.mouse_mode = if set { MouseMode::ButtonMotion } else { MouseMode::Off };
            }
            // Mouse button + all motion.
            1003 => {
                self.mouse_mode = if set { MouseMode::AnyMotion } else { MouseMode::Off };
            }
            // SGR extended mouse encoding.
            1006 => { self.mouse_sgr = set; }
            // URXVT extended mouse encoding — we use SGR, treat as no-op.
            1015 => {}

            // Alternate screen — save/restore cursor + grid.
            47 => {
                if set {
                    self.in_alt = true;
                    let rows  = self.alt.rows;
                    let attrs = self.attrs.clone();
                    for r in 0..rows { self.alt.erase_row(r, &attrs); }
                } else {
                    self.in_alt = false;
                }
                self.pending_wrap = false;
            }
            1048 => {
                if set {
                    self.saved_cursor = Some(self.cursor.clone());
                } else if let Some(c) = self.saved_cursor.clone() {
                    self.cursor = c;
                }
            }
            1049 => {
                if set {
                    self.saved_cursor = Some(self.cursor.clone());
                    self.in_alt = true;
                    self.cursor = Cursor::default();
                    let rows  = self.alt.rows;
                    let attrs = self.attrs.clone();
                    for r in 0..rows { self.alt.erase_row(r, &attrs); }
                } else {
                    self.in_alt = false;
                    if let Some(c) = self.saved_cursor.take() { self.cursor = c; }
                }
                self.pending_wrap = false;
            }

            // Bracketed paste mode.
            2004 => { self.bracketed_paste = set; }

            // Synchronized output (DEC mode 2026): pause rendering mid-update.
            2026 => { self.sync_output = set; }

            _ => {}
        }
    }

    fn apply_sgr(&mut self, params: &[u32]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0  => self.attrs = Attrs::default(),
                1  => self.attrs.bold = true,
                3  => self.attrs.italic = true,
                4  => self.attrs.underline = true,
                7  => self.attrs.inverse = true,
                22 => self.attrs.bold = false,
                23 => self.attrs.italic = false,
                24 => self.attrs.underline = false,
                27 => self.attrs.inverse = false,
                n @ 30..=37  => self.attrs.fg = ANSI_NAMED[(n - 30) as usize],
                39           => self.attrs.fg = Color::Default,
                n @ 40..=47  => self.attrs.bg = ANSI_NAMED[(n - 40) as usize],
                49           => self.attrs.bg = Color::Default,
                n @ 90..=97  => self.attrs.fg = ANSI_BRIGHT[(n - 90) as usize],
                n @ 100..=107 => self.attrs.bg = ANSI_BRIGHT[(n - 100) as usize],
                38 => {
                    if let Some(color) = parse_extended_color(params, &mut i) {
                        self.attrs.fg = color;
                    }
                }
                48 => {
                    if let Some(color) = parse_extended_color(params, &mut i) {
                        self.attrs.bg = color;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    pub fn debug_dump(&self) -> String {
        let mut out = String::new();
        for row in 0..self.rows() {
            let line: String = self.row(row).iter().map(|c| c.ch).collect();
            let trimmed = line.trim_end_matches(' ');
            out.push_str(trimmed);
            out.push('\n');
        }
        out
    }
}

fn parse_extended_color(params: &[u32], i: &mut usize) -> Option<Color> {
    match params.get(*i + 1).copied() {
        Some(5) => {
            // 256-colour: 38;5;n
            let n = params.get(*i + 2).copied()? as u8;
            *i += 2;
            Some(Color::Palette(n))
        }
        Some(2) => {
            // Truecolor: 38;2;r;g;b
            let r = params.get(*i + 2).copied()? as u8;
            let g = params.get(*i + 3).copied()? as u8;
            let b = params.get(*i + 4).copied()? as u8;
            *i += 4;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn feed(screen: &mut Screen, input: &[u8]) {
        let mut p = Parser::new();
        p.advance(input, &mut |a| screen.process(a));
    }

    // ── existing tests ────────────────────────────────────────────────────────

    #[test]
    fn cursor_movement_basic() {
        let mut s = Screen::new(24, 80);
        feed(&mut s, b"\x1b[5;10H"); // CUP row=5, col=10
        assert_eq!(s.cursor().row, 4);
        assert_eq!(s.cursor().col, 9);
    }

    #[test]
    fn sgr_colors_16() {
        let mut s = Screen::new(24, 80);
        feed(&mut s, b"\x1b[31m"); // red fg
        assert_eq!(s.attrs.fg, ANSI_NAMED[1]);
    }

    #[test]
    fn sgr_256_color() {
        let mut s = Screen::new(24, 80);
        feed(&mut s, b"\x1b[38;5;200m");
        assert_eq!(s.attrs.fg, Color::Palette(200));
    }

    #[test]
    fn sgr_truecolor() {
        let mut s = Screen::new(24, 80);
        feed(&mut s, b"\x1b[38;2;10;20;30m");
        assert_eq!(s.attrs.fg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn alt_screen_isolates_scrollback() {
        let mut s = Screen::new(5, 10);
        // Fill normal screen, scroll some lines.
        for _ in 0..10 {
            feed(&mut s, b"hello\r\n");
        }
        let sb_before = s.scrollback().len();
        // Enter alt screen and write something.
        feed(&mut s, b"\x1b[?1049h");
        feed(&mut s, b"alt\r\n");
        assert_eq!(s.scrollback().len(), sb_before);
        // Leave alt screen: scrollback unchanged.
        feed(&mut s, b"\x1b[?1049l");
        assert_eq!(s.scrollback().len(), sb_before);
    }

    // ── Phase 6 new tests ─────────────────────────────────────────────────────

    #[test]
    fn app_cursor_keys_mode() {
        let mut s = Screen::new(24, 80);
        assert!(!s.app_cursor_keys);
        feed(&mut s, b"\x1b[?1h");
        assert!(s.app_cursor_keys);
        feed(&mut s, b"\x1b[?1l");
        assert!(!s.app_cursor_keys);
    }

    #[test]
    fn mouse_mode_tracking() {
        let mut s = Screen::new(24, 80);
        assert_eq!(s.mouse_mode, MouseMode::Off);
        feed(&mut s, b"\x1b[?1000h");
        assert_eq!(s.mouse_mode, MouseMode::X10);
        feed(&mut s, b"\x1b[?1002h");
        assert_eq!(s.mouse_mode, MouseMode::ButtonMotion);
        feed(&mut s, b"\x1b[?1003h");
        assert_eq!(s.mouse_mode, MouseMode::AnyMotion);
        feed(&mut s, b"\x1b[?1000l");
        assert_eq!(s.mouse_mode, MouseMode::Off);
    }

    #[test]
    fn sgr_mouse_encoding_flag() {
        let mut s = Screen::new(24, 80);
        assert!(!s.mouse_sgr);
        feed(&mut s, b"\x1b[?1006h");
        assert!(s.mouse_sgr);
        feed(&mut s, b"\x1b[?1006l");
        assert!(!s.mouse_sgr);
    }

    #[test]
    fn bracketed_paste_mode() {
        let mut s = Screen::new(24, 80);
        assert!(!s.bracketed_paste);
        feed(&mut s, b"\x1b[?2004h");
        assert!(s.bracketed_paste);
        feed(&mut s, b"\x1b[?2004l");
        assert!(!s.bracketed_paste);
    }

    #[test]
    fn deckpam_deckpnm() {
        let mut s = Screen::new(24, 80);
        assert!(!s.app_keypad);
        feed(&mut s, b"\x1b="); // DECKPAM
        assert!(s.app_keypad);
        feed(&mut s, b"\x1b>"); // DECKPNM
        assert!(!s.app_keypad);
    }

    #[test]
    fn osc_window_title() {
        let mut s = Screen::new(24, 80);
        assert!(s.window_title.is_none());
        feed(&mut s, b"\x1b]2;my shell\x07");
        assert_eq!(s.window_title.as_deref(), Some("my shell"));
        // OSC 0 also sets title.
        feed(&mut s, b"\x1b]0;new title\x07");
        assert_eq!(s.window_title.as_deref(), Some("new title"));
    }

    #[test]
    fn decscusr_cursor_shape() {
        let mut s = Screen::new(24, 80);
        assert_eq!(s.cursor_shape, 0);
        feed(&mut s, b"\x1b[2 q"); // steady block
        assert_eq!(s.cursor_shape, 2);
        feed(&mut s, b"\x1b[6 q"); // steady bar
        assert_eq!(s.cursor_shape, 6);
    }

    #[test]
    fn rep_repeats_last_char() {
        let mut s = Screen::new(24, 80);
        feed(&mut s, b"A\x1b[4b"); // 'A' then REP 4 (total 5 A's)
        assert_eq!(s.cell(0, 0).ch, 'A');
        assert_eq!(s.cell(0, 1).ch, 'A');
        assert_eq!(s.cell(0, 4).ch, 'A');
    }

    #[test]
    fn dsr_cursor_pos_queues_response() {
        let mut s = Screen::new(24, 80);
        feed(&mut s, b"\x1b[5;10H"); // cursor to row 5 col 10
        feed(&mut s, b"\x1b[6n");    // request CPR
        assert!(!s.pending_responses.is_empty());
        let resp = std::str::from_utf8(&s.pending_responses[0]).unwrap();
        assert_eq!(resp, "\x1b[5;10R");
    }

    // ── Phase 7 tests ─────────────────────────────────────────────────────────

    #[test]
    fn apc_kitty_image_parsed_cleanly() {
        // APC payload starting with 'G' (Kitty inline-image command).
        // Should parse without corrupting any grid cells.
        let mut s = Screen::new(24, 80);
        feed(&mut s, b"before");
        feed(&mut s, b"\x1b_Ga=T,f=32,s=4,v=4,m=0;AAAA\x1b\\");
        feed(&mut s, b"after");
        // Grid must have "before" and "after" text intact; APC leaves no cell residue.
        assert_eq!(s.cell(0, 0).ch, 'b');
        assert_eq!(s.cell(0, 6).ch, 'a');
    }

    #[test]
    fn synchronized_output_mode_2026() {
        let mut s = Screen::new(24, 80);
        assert!(!s.sync_output);
        feed(&mut s, b"\x1b[?2026h"); // enable synchronized output
        assert!(s.sync_output);
        feed(&mut s, b"\x1b[?2026l"); // disable
        assert!(!s.sync_output);
    }

    #[test]
    fn kitty_keyboard_protocol_push_pop_query() {
        let mut s = Screen::new(24, 80);
        assert!(s.keyboard_modes.is_empty());
        // Push flags=1 (disambiguate)
        feed(&mut s, b"\x1b[>1u");
        assert_eq!(s.keyboard_modes, vec![1]);
        // Push flags=3 (disambiguate + report event types)
        feed(&mut s, b"\x1b[>3u");
        assert_eq!(s.keyboard_modes, vec![1, 3]);
        // Query — should respond with current top (3)
        s.pending_responses.clear();
        feed(&mut s, b"\x1b[?u");
        assert!(!s.pending_responses.is_empty());
        let resp = std::str::from_utf8(&s.pending_responses[0]).unwrap();
        assert_eq!(resp, "\x1b[?3u");
        // Pop 1 entry
        feed(&mut s, b"\x1b[<1u");
        assert_eq!(s.keyboard_modes, vec![1]);
        // Pop remaining
        feed(&mut s, b"\x1b[<1u");
        assert!(s.keyboard_modes.is_empty());
    }
}
