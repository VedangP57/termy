use std::collections::VecDeque;

use crate::cell::{Attrs, Cell};
use crate::color::{Color, ANSI_BRIGHT, ANSI_NAMED};
use crate::parser::Action;

const MAX_SCROLLBACK: usize = 10_000;

/// Cursor state.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
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
    normal: Grid,
    alt: Grid,
    in_alt: bool,
    scrollback: VecDeque<Vec<Cell>>,
    cursor: Cursor,
    saved_cursor: Option<Cursor>,
    pub attrs: Attrs,
    scroll_top: usize,
    scroll_bottom: usize,
    pending_wrap: bool,
}

impl Screen {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            normal: Grid::new(rows, cols),
            alt: Grid::new(rows, cols),
            in_alt: false,
            scrollback: VecDeque::new(),
            cursor: Cursor::default(),
            saved_cursor: None,
            attrs: Attrs::default(),
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            pending_wrap: false,
        }
    }

    pub fn rows(&self) -> usize { self.active().rows }
    pub fn cols(&self) -> usize { self.active().cols }
    pub fn cursor(&self) -> &Cursor { &self.cursor }
    pub fn scrollback(&self) -> &VecDeque<Vec<Cell>> { &self.scrollback }
    pub fn is_in_alt(&self) -> bool { self.in_alt }

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

    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.normal.resize(rows, cols);
        self.alt.resize(rows, cols);
        self.scroll_top = 0;
        self.scroll_bottom = rows.saturating_sub(1);
        self.cursor.row = self.cursor.row.min(rows.saturating_sub(1));
        self.cursor.col = self.cursor.col.min(cols.saturating_sub(1));
    }

    pub fn process(&mut self, action: Action) {
        match action {
            Action::Print(ch) => self.print(ch),
            Action::Execute(b) => self.execute(b),
            Action::CsiDispatch { params, private_marker, intermediates: _, final_byte } => {
                self.csi(params, private_marker, final_byte);
            }
            Action::EscDispatch { intermediates, byte } => {
                self.esc(intermediates, byte);
            }
            Action::OscDispatch(_) => {}
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
        // Snapshot attrs before active_mut borrow.
        let attrs = self.attrs.clone();
        let cell = self.active_mut().cell_mut(row, col);
        cell.ch = ch;
        cell.attrs = attrs;
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
            // Snapshot fields before active_mut borrow.
            let top = self.scroll_top;
            let bottom = self.scroll_bottom;
            let attrs = self.attrs.clone();
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
            _ => {}
        }
    }

    fn csi(&mut self, mut params: Vec<u32>, private_marker: Option<u8>, final_byte: u8) {
        fn p(params: &[u32], idx: usize) -> u32 {
            params.get(idx).copied().unwrap_or(0)
        }
        fn p1(params: &[u32], idx: usize) -> u32 {
            let v = params.get(idx).copied().unwrap_or(0);
            if v == 0 { 1 } else { v }
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
                // Snapshot before active_mut.
                let attrs = self.attrs.clone();
                let rows = self.rows();
                let row = self.cursor.row;
                let col = self.cursor.col;
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
                let row = self.cursor.row;
                let col = self.cursor.col;
                match p(&params, 0) {
                    0 => self.active_mut().erase_row_from(row, col, &attrs),
                    1 => self.active_mut().erase_row_before(row, col, &attrs),
                    2 => self.active_mut().erase_row(row, &attrs),
                    _ => {}
                }
            }

            // ── Scroll ───────────────────────────────────────────────────────
            (None, b'S') => {
                let n = p1(&params, 0) as usize;
                let top = self.scroll_top;
                let bottom = self.scroll_bottom;
                let attrs = self.attrs.clone();
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
                let n = p1(&params, 0) as usize;
                let top = self.scroll_top;
                let bottom = self.scroll_bottom;
                let attrs = self.attrs.clone();
                self.active_mut().scroll_down(top, bottom, n, &attrs);
            }

            // ── Insert / delete ──────────────────────────────────────────────
            (None, b'L') => {
                let n = p1(&params, 0) as usize;
                let bottom = self.scroll_bottom;
                let row = self.cursor.row;
                let attrs = self.attrs.clone();
                self.active_mut().insert_lines(row, bottom, n, &attrs);
            }
            (None, b'M') => {
                let n = p1(&params, 0) as usize;
                let bottom = self.scroll_bottom;
                let row = self.cursor.row;
                let attrs = self.attrs.clone();
                self.active_mut().delete_lines(row, bottom, n, &attrs);
            }
            (None, b'@') => {
                let n = p1(&params, 0) as usize;
                let (row, col) = (self.cursor.row, self.cursor.col);
                let attrs = self.attrs.clone();
                self.active_mut().insert_chars(row, col, n, &attrs);
            }
            (None, b'P') => {
                let n = p1(&params, 0) as usize;
                let (row, col) = (self.cursor.row, self.cursor.col);
                let attrs = self.attrs.clone();
                self.active_mut().delete_chars(row, col, n, &attrs);
            }
            (None, b'X') => {
                let n = p1(&params, 0) as usize;
                let (row, col) = (self.cursor.row, self.cursor.col);
                let cols = self.cols();
                let end = (col + n).min(cols);
                let attrs = self.attrs.clone();
                for c in col..end {
                    *self.active_mut().cell_mut(row, c) = Cell::blank_with(attrs.clone());
                }
            }

            // ── Scroll region ────────────────────────────────────────────────
            (None, b'r') => {
                let rows = self.rows();
                let top = p1(&params, 0) as usize - 1;
                let bottom = if p(&params, 1) == 0 { rows - 1 } else { p(&params, 1) as usize - 1 };
                if top < bottom && bottom < rows {
                    self.scroll_top = top;
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

            // ── Private modes ────────────────────────────────────────────────
            (Some(b'?'), b'h') => {
                for mode in params.clone() { self.set_private_mode(mode, true); }
            }
            (Some(b'?'), b'l') => {
                for mode in params.clone() { self.set_private_mode(mode, false); }
            }

            _ => {}
        }
    }

    fn set_private_mode(&mut self, mode: u32, set: bool) {
        match mode {
            25 => { self.cursor.visible = set; }
            1049 => {
                if set {
                    self.saved_cursor = Some(self.cursor.clone());
                    self.in_alt = true;
                    self.cursor = Cursor::default();
                    let rows = self.alt.rows;
                    let attrs = self.attrs.clone();
                    for r in 0..rows {
                        self.alt.erase_row(r, &attrs);
                    }
                } else {
                    self.in_alt = false;
                    if let Some(c) = self.saved_cursor.take() { self.cursor = c; }
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
            47 => {
                if set {
                    self.in_alt = true;
                    let rows = self.alt.rows;
                    let attrs = self.attrs.clone();
                    for r in 0..rows {
                        self.alt.erase_row(r, &attrs);
                    }
                } else {
                    self.in_alt = false;
                }
                self.pending_wrap = false;
            }
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
                n @ 30..=37 => self.attrs.fg = ANSI_NAMED[(n - 30) as usize],
                39 => self.attrs.fg = Color::Default,
                n @ 40..=47 => self.attrs.bg = ANSI_NAMED[(n - 40) as usize],
                49 => self.attrs.bg = Color::Default,
                n @ 90..=97  => self.attrs.fg = ANSI_BRIGHT[(n - 90) as usize],
                n @ 100..=107 => self.attrs.bg = ANSI_BRIGHT[(n - 100) as usize],
                38 => {
                    if let Some(color) = parse_extended_color(params, &mut i) {
                        self.attrs.fg = color;
                    }
                    continue;
                }
                48 => {
                    if let Some(color) = parse_extended_color(params, &mut i) {
                        self.attrs.bg = color;
                    }
                    continue;
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
            let idx = params.get(*i + 2).copied()? as u8;
            *i += 3;
            Some(Color::Palette(idx))
        }
        Some(2) => {
            let r = params.get(*i + 2).copied()? as u8;
            let g = params.get(*i + 3).copied()? as u8;
            let b = params.get(*i + 4).copied()? as u8;
            *i += 5;
            Some(Color::Rgb(r, g, b))
        }
        _ => {
            *i += 1;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn screen_from(input: &[u8], rows: usize, cols: usize) -> Screen {
        let mut s = Screen::new(rows, cols);
        let mut p = Parser::new();
        p.advance(input, &mut |a| s.process(a));
        s
    }

    // ── Color parsing ─────────────────────────────────────────────────────────

    #[test]
    fn sgr_standard_fg() {
        let s = screen_from(b"\x1b[31mX", 1, 5);
        assert_eq!(s.cell(0, 0).attrs.fg, Color::Indexed(1));
    }

    #[test]
    fn sgr_bright_fg() {
        let s = screen_from(b"\x1b[91mX", 1, 5);
        assert_eq!(s.cell(0, 0).attrs.fg, Color::Indexed(9));
    }

    #[test]
    fn sgr_256_fg() {
        let s = screen_from(b"\x1b[38;5;200mX", 1, 5);
        assert_eq!(s.cell(0, 0).attrs.fg, Color::Palette(200));
    }

    #[test]
    fn sgr_truecolor_fg() {
        let s = screen_from(b"\x1b[38;2;100;150;200mX", 1, 5);
        assert_eq!(s.cell(0, 0).attrs.fg, Color::Rgb(100, 150, 200));
    }

    #[test]
    fn sgr_bold_italic_underline() {
        let s = screen_from(b"\x1b[1;3;4mX", 1, 5);
        let a = &s.cell(0, 0).attrs;
        assert!(a.bold && a.italic && a.underline);
    }

    #[test]
    fn sgr_reset() {
        let s = screen_from(b"\x1b[1m\x1b[0mX", 1, 5);
        assert!(!s.cell(0, 0).attrs.bold);
    }

    // ── Cursor movement ───────────────────────────────────────────────────────

    #[test]
    fn cursor_up_from_row4() {
        // CUP to row 5 col 1 → row=4, then CUU 2 → row=2.
        let s = screen_from(b"\x1b[5;1H\x1b[2A", 10, 10);
        assert_eq!(s.cursor().row, 2);
    }

    #[test]
    fn cursor_set_position() {
        let s = screen_from(b"\x1b[3;7H", 10, 10);
        assert_eq!(s.cursor().row, 2);
        assert_eq!(s.cursor().col, 6);
    }

    #[test]
    fn cursor_home_default() {
        let s = screen_from(b"\x1b[5;5H\x1b[H", 10, 10);
        assert_eq!(s.cursor().row, 0);
        assert_eq!(s.cursor().col, 0);
    }

    // ── Erase ─────────────────────────────────────────────────────────────────

    #[test]
    fn erase_line_to_end() {
        // "hello", move to col 2, erase to end of line → "he   "
        let s = screen_from(b"hello\x1b[1;3H\x1b[K", 3, 10);
        assert_eq!(s.cell(0, 0).ch, 'h');
        assert_eq!(s.cell(0, 1).ch, 'e');
        assert_eq!(s.cell(0, 2).ch, ' ');
    }

    #[test]
    fn erase_whole_line() {
        let s = screen_from(b"hello\x1b[1;1H\x1b[2K", 3, 10);
        let row: String = s.row(0).iter().map(|c| c.ch).collect();
        assert!(row.chars().all(|c| c == ' '));
    }

    #[test]
    fn erase_display_to_end() {
        let s = screen_from(b"hello\x1b[1;3H\x1b[0J", 3, 10);
        let row1: String = s.row(1).iter().map(|c| c.ch).collect();
        assert!(row1.chars().all(|c| c == ' '));
    }

    // ── Alt screen ────────────────────────────────────────────────────────────

    #[test]
    fn alt_screen_enter_exit_restores_normal() {
        let mut input = Vec::new();
        input.extend_from_slice(b"line1\r\nline2\r\n");
        input.extend_from_slice(b"\x1b[?1049h"); // enter alt
        input.extend_from_slice(b"altcontent");
        input.extend_from_slice(b"\x1b[?1049l"); // exit alt
        let s = screen_from(&input, 5, 20);
        assert!(!s.is_in_alt());
        let found_normal = (0..s.rows()).any(|r| {
            let text: String = s.row(r).iter().map(|c| c.ch).collect();
            text.contains("line")
        }) || s.scrollback().iter().any(|row| {
            let text: String = row.iter().map(|c| c.ch).collect();
            text.contains("line")
        });
        assert!(found_normal, "normal-screen content missing after alt-screen exit");
        let alt_leaked = (0..s.rows()).any(|r| {
            let text: String = s.row(r).iter().map(|c| c.ch).collect();
            text.contains("altcontent")
        });
        assert!(!alt_leaked, "alt-screen content leaked into normal screen");
    }

    // ── Scrollback ────────────────────────────────────────────────────────────

    #[test]
    fn scrollback_accumulates() {
        let mut input = Vec::new();
        for i in 0..5u8 {
            input.push(b'A' + i);
            input.extend_from_slice(b"\r\n");
        }
        let s = screen_from(&input, 3, 10);
        assert!(s.scrollback().len() >= 2, "expected scrollback, got {}", s.scrollback().len());
    }
}
