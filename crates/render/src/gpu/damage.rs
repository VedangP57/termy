// Damage tracking: diff the current terminal grid against the previous frame's
// snapshot, exposing per-cell dirty flags and frame-level counters.

use vt::{Attrs, Terminal};

#[derive(Clone, PartialEq)]
struct CellSnapshot {
    ch: char,
    attrs: Attrs,
}

pub struct DamageTracker {
    prev:           Vec<CellSnapshot>,
    pub dirty:      Vec<bool>,
    // Counters exposed for the debug/verify acceptance criterion.
    pub rendered:   u64,
    pub skipped:    u64,
}

impl DamageTracker {
    pub fn new() -> Self {
        Self { prev: Vec::new(), dirty: Vec::new(), rendered: 0, skipped: 0 }
    }

    /// Diff `term` against the previous snapshot.
    /// Returns true when at least one cell changed (i.e. a render pass is needed).
    /// Populates `self.dirty` with per-cell flags indexed by `row * cols + col`.
    /// Clear the snapshot, forcing a full redraw on the next `diff`.
    pub fn reset(&mut self) {
        self.prev.clear();
    }

    pub fn diff(&mut self, term: &Terminal) -> bool {
        let rows = term.screen.rows();
        let cols = term.screen.cols();
        let total = rows * cols;

        // Resize on grid change (always triggers a full redraw).
        if self.prev.len() != total {
            let empty = CellSnapshot { ch: '\0', attrs: Attrs::default() };
            self.prev.resize(total, empty);
            self.dirty.resize(total, true);
            return true;
        }

        let mut any = false;
        for row in 0..rows {
            for col in 0..cols {
                let idx = row * cols + col;
                let cell = term.screen.cell(row, col);
                let snap = CellSnapshot { ch: cell.ch, attrs: cell.attrs.clone() };
                if self.prev[idx] != snap {
                    self.prev[idx] = snap;
                    self.dirty[idx] = true;
                    any = true;
                } else {
                    self.dirty[idx] = false;
                }
            }
        }
        any
    }
}
