# Progress log

After each phase commit, append an entry here. This is what the project owner
reads when checking in — not the full git log.

Format per entry:
- **Phase:** number and name
- **Built:** what was implemented
- **Deviations:** anything that differed from the docs and why (none = say so)
- **Moved to QUESTIONS.md:** any open decisions surfaced

---

<!-- entries go below this line -->

## Phase 2 — VT/ANSI parser + grid model

- **Phase:** 2 — VT parsing into a structured grid
- **Built:**
  - `crates/vt/src/color.rs`: `Color` enum — Default, Indexed(u8), Palette(u8), Rgb(u8,u8,u8)
  - `crates/vt/src/cell.rs`: `Cell` (char + Attrs) and `Attrs` (fg, bg, bold, italic, underline, inverse)
  - `crates/vt/src/parser.rs`: Paul Williams VT500 state machine — Ground/Escape/CsiEntry/CsiParam/CsiIntermediate/CsiIgnore/OscString states; emits `Action` enum; UTF-8 multi-byte reassembly; 9 unit tests
  - `crates/vt/src/grid.rs`: `Grid` (2D cell array with scroll/erase/insert/delete primitives) + `Screen` (normal+alt grids, scrollback VecDeque, cursor, scroll region, SGR, private modes); 14 unit tests covering color (16/256/truecolor), cursor movement, erase-line/display, alt-screen enter/exit, scrollback
  - `crates/vt/src/lib.rs`: `Terminal` public API — `advance(bytes)`, `resize()`, `last_line_text()`
  - `crates/agentd`: `GridDetector` replaces `NaiveDetector` — reads from parsed grid instead of raw bytes; 6 unit tests
  - `crates/server-bin`: updated to use `GridDetector`
- **Acceptance criteria:**
  - `cargo build --workspace` — clean ✓
  - `cargo test --workspace` — 29/29 pass ✓
  - Color parsing: 16-color, 256-color, truecolor all tested ✓
  - Cursor movement (CUU/CUD/CUF/CUB/CUP/HVP) tested ✓
  - Erase-line (EL) and erase-display (ED) tested ✓
  - Alt-screen enter/exit (1049h/l) with scrollback isolation tested ✓
  - `cargo tree -p server-bin` — no render/fonts/wgpu ✓
- **Deviations:** None. Grid written independently; termwiz/wezterm-term used only as reading reference, not imported.
- **Moved to QUESTIONS.md:** None new. Existing Q1 (socket path) and Q2 (binary structure) unchanged.

## Phase 1 — PTY skeleton, naive agent detector, client/server skeleton

- **Phase:** 1 — PTY + agent state stub + binary skeleton
- **Built:**
  - `crates/pty`: `PtyHandle` wrapping `portable-pty` — spawns `$SHELL`, splits reader/writer for threading
  - `crates/agentd`: `PaneState` enum (Idle/Working/Blocked/Done), `NaiveDetector` — heuristic prompt-suffix matching on raw PTY bytes with ANSI strip; 5 unit tests all pass
  - `crates/server-bin`: `run(socket_path)` — opens PTY, binds Unix socket, accepts one client, two threads bidirectional relay with `NaiveDetector` side-channel logging state transitions
  - `crates/client-bin` (`termd`): arg dispatch — `--server` → `server_bin::run()`, `--attach` → Phase 8 stub, default → `passthrough::run()`
  - `crates/client-bin/passthrough.rs`: direct PTY mode, two threads: PTY→stdout, stdin→PTY
  - Stub crates: `vt`, `render`, `fonts`, `protocol` — empty lib.rs with phase-gating comments
- **Acceptance criteria:**
  - `cargo build --workspace` — clean ✓
  - `cargo test --workspace` — 5/5 pass ✓
  - `cargo tree -p server-bin` — no render/fonts/wgpu in tree ✓
  - Single binary `termd` produced ✓
- **Deviations:**
  - `NaiveDetector.state` is `Option<PaneState>` (None = unclassified) rather than defaulting to `Idle`. This ensures the first `feed()` always returns `Some(state)`. No spec violation — the spec doesn't prescribe the initial value.
- **Moved to QUESTIONS.md:** Q1 (socket path convention), Q2 (server-bin as library crate confirmed)
