// Written independently; termwiz/wezterm-term used as reference only, never as a dependency.

pub mod cell;
pub mod color;
pub mod grid;
pub mod parser;

pub use cell::{Attrs, Cell};
pub use color::Color;
pub use grid::{Cursor, MouseMode, Screen};
pub use parser::{Action, Parser};

/// A complete terminal: parser + screen state. Feed raw bytes; read the grid.
pub struct Terminal {
    parser: Parser,
    pub screen: Screen,
}

impl Terminal {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            parser: Parser::new(),
            screen: Screen::new(rows, cols),
        }
    }

    /// Feed raw PTY bytes.
    pub fn advance(&mut self, bytes: &[u8]) {
        let screen = &mut self.screen;
        self.parser.advance(bytes, &mut |action| screen.process(action));
    }

    /// Resize the terminal grid.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.screen.resize(rows, cols);
    }

    /// Drain queued terminal responses (DSR, DA) that must be written back to the PTY.
    pub fn drain_responses(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.screen.pending_responses)
    }

    /// Return the text content of the last non-blank row on the active screen.
    /// Trailing spaces are preserved so prompt-suffix matching works (e.g. "$ ").
    pub fn last_line_text(&self) -> String {
        for row in (0..self.screen.rows()).rev() {
            let text: String = self.screen.row(row).iter().map(|c| c.ch).collect();
            if !text.trim().is_empty() {
                return text;
            }
        }
        String::new()
    }
}
