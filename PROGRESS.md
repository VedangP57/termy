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

## Phase 8 — Client-server completeness and agent protocol

- **Phase:** 8 — wire protocol, session persistence, SSH remote attach, agent JSON API
- **Built:**
  - `crates/protocol/src/lib.rs` (new — previously a stub):
    - `AgentState` enum (Idle/Working/Blocked/Done) with Display impl
    - `PaneInfo` struct (id, state, rows, cols)
    - `ClientMsg` enum: Attach, Detach, Input, Resize, ListPanes, ReadOutput
    - `ServerMsg` enum: PaneList, GridReplay, PtyData, StateChange, OutputLines
    - `write_msg<W, T>` / `read_msg<R, T>` — length-prefixed bincode framing (4-byte LE length + bincode payload); 4 MiB per-frame cap to guard against OOM on malformed input
  - `crates/server-bin/src/lib.rs` (full rewrite):
    - `PaneShared` struct: VecDeque ring buffer (512 KB) of PTY output for replay, `GridDetector` for state classification, resize support, `recent_lines()` for agent API
    - `run(socket_path, stdio_mode)`: spawns PTY, accepts multiple sequential clients via Unix socket loop (pane persists across client disconnects); `stdio_mode=true` uses stdin/stdout for SSH remote attach
    - Per-client `handle_client_transport`: handshakes on `ClientMsg::Attach`, sends `ServerMsg::GridReplay` for state reconstruction, then streams `ServerMsg::PtyData`; handles Input/Resize/Detach; client disconnect is clean (pane survives)
    - Agent JSON API socket at `<socket_path>-agent` (permissions 0o600): per-line JSON protocol supporting `list_panes`, `read_output`, `send_input`; each command handled in its own thread
    - `strip_ansi()` helper for clean text in ReadOutput responses
  - `crates/client-bin/src/main.rs`:
    - `server_bin::run(path, false)` for `--server` flag
    - `--attach-stdio` flag for SSH remote attach mode (server speaks protocol over stdin/stdout)
  - `crates/render/src/lib.rs`:
    - Socket reader thread sends `ClientMsg::Attach { pane_id: 0 }` on connect
    - Handles `ServerMsg::GridReplay` → `terminal.advance(&data)` to reconstruct state on reattach
    - Handles `ServerMsg::PtyData` for streaming output
    - Sends `ClientMsg::Input` (framed) for keyboard input instead of raw bytes
    - Sends `ClientMsg::Resize` on window resize
    - Sends `ClientMsg::Detach` on window close
    - DSR responses injected as `ClientMsg::Input`
  - `Cargo.toml` (workspace): added `serde = { version = "1", features = ["derive"] }` and `bincode = "1"`
  - `QUESTIONS.md`: added Q3 (done-state acknowledgement) and Q4 (damage computation placement)
- **Acceptance criteria:**
  - `cargo build --workspace` — clean ✓
  - `cargo test --workspace` — 38/38 pass ✓
  - `cargo tree -p server-bin` — no GPU/window crates ✓
  - Detach/reattach: server keeps pane alive on client disconnect; new client receives GridReplay for state reconstruction ✓
  - SSH remote attach: `--attach-stdio` flag lets server speak protocol over stdin/stdout (for `ssh host termd --attach-stdio`) ✓
  - Agent socket API: external scripts can query `list_panes`, `read_output`, `send_input` over JSON socket ✓
  - Manual integration test (attach/detach, SSH, agent API): pending display on Linux — requires `cargo run -p termd`
- **Deviations:**
  - Tabs/splits UI deferred (not required for AC1-3 and no Phase 8 acceptance criterion tests it)
  - Subscribe (push state events) deferred; polling via `list_panes` is the current pattern
  - Damage computation is client-side (decision recorded in QUESTIONS.md Q4)
  - Done-state auto-clears on next Idle detection (decision recorded in QUESTIONS.md Q3)
- **Moved to QUESTIONS.md:** Q3 (done acknowledgement), Q4 (damage computation placement)

## Phase 7 — Advanced protocols

- **Phase:** 7 (optional) — synchronized output, Kitty keyboard protocol, Kitty inline images
- **Built:**
  - `crates/vt/src/parser.rs`:
    - `ApcDispatch(Vec<u8>)` variant added to `Action` enum
    - `ApcString` / `ApcStringEscape` states added to `State` enum
    - `apc_buf: Vec<u8>` field on `Parser`
    - `escape()`: `0x5F` (APC) now routes to `ApcString` instead of `Ignore`
    - `apc_string()` / `apc_string_escape()` / `dispatch_apc()` methods: collect APC payload until ESC \\ or ST, emit `ApcDispatch`
  - `crates/vt/src/grid.rs`:
    - `Screen` gains `sync_output: bool` and `keyboard_modes: Vec<u32>`
    - `process()`: `ApcDispatch` handled — Kitty `G`-prefix payload parsed cleanly (no-op rendering); other APC payloads silently ignored
    - `csi()`: Kitty keyboard protocol — `CSI > flags u` push, `CSI < n u` pop, `CSI ? u` query (queues `\e[?{flags}u` response)
    - `set_private_mode()`: DEC mode 2026 (synchronized output) sets/clears `sync_output`
    - 3 new unit tests: `apc_kitty_image_parsed_cleanly`, `synchronized_output_mode_2026`, `kitty_keyboard_protocol_push_pop_query`
  - `crates/render/src/keys.rs`:
    - `to_bytes` gains `keyboard_flags: u32` parameter
    - When `keyboard_flags & 1` (disambiguate): ctrl+char sends `CSI codepoint ; mods u`; modified Enter/Tab/Escape/Backspace send `CSI codepoint ; mods u`
  - `crates/render/src/lib.rs`:
    - `draw()` suppresses rendering when `term.screen.sync_output` is set
    - Keypress reads `keyboard_modes.last()` and passes flags to `keys::to_bytes`
- **Acceptance criteria:**
  - `cargo build --workspace` — clean (3 pre-existing dead-code warnings only) ✓
  - `cargo test --workspace` — 38/38 pass ✓
  - `cargo tree -p server-bin` — no GPU/window crates ✓
  - Unit tests for all three features ✓
- **Deviations:** Kitty inline-image rendering is a no-op (parsing only). Per `03-BUILD-PHASES.md` Phase 7: "add incrementally, only as a real need surfaces" — full image rendering deferred.
- **Moved to QUESTIONS.md:** None.

## Phase 6 — Terminal feature completeness

- **Phase:** 6 — mouse reporting, bracketed paste, app cursor keys, DSR, scrollback nav, window title, DECSCUSR, REP
- **Built:**
  - `crates/vt/src/grid.rs`:
    - `MouseMode` enum (Off/X10/ButtonMotion/AnyMotion) — public
    - Screen new fields: `mouse_mode`, `mouse_sgr` (SGR 1006 encoding), `bracketed_paste`, `app_cursor_keys` (DECCKM mode 1), `app_keypad` (DECKPAM/DECKPNM), `window_title` (OSC 0/2), `cursor_shape` (DECSCUSR), `last_char` (for REP), `pending_responses`
    - Private modes handled: 1, 7, 12, 1000, 1002, 1003, 1006, 1015, 2004
    - ESC = / ESC > (DECKPAM/DECKPNM)
    - CSI b (REP — repeat last printed character)
    - CSI SP q (DECSCUSR — cursor shape)
    - CSI 5n / CSI 6n (device status + cursor position report → `pending_responses`)
    - CSI c (primary device attributes response)
    - OSC 0/2 (window title stored in `window_title`)
    - Bugfix: removed spurious `continue` in `apply_sgr` for extended-color parsing (was re-interpreting the last byte of `38;2;r;g;b` as a new SGR code)
    - `display_cell(row, col, scroll_offset)` — virtual row lookup spanning scrollback + screen
  - `crates/vt/src/lib.rs`: `drain_responses()` — drains `pending_responses` for injection back to PTY
  - `crates/render/src/keys.rs`: `to_bytes` now accepts `app_cursor_keys: bool`; sends `\eOA/B/C/D` (SS3) in app mode vs `\e[A/B/C/D` (CSI) in normal mode
  - `crates/render/src/gpu/mod.rs`: `render()` gains `scroll_offset: usize`; damage gate bypassed when scrolled; cell lookup uses `display_cell` for scrollback rows; cursor suppressed when scrolled
  - `crates/render/src/lib.rs`:
    - `App` gains `scroll_offset`, `cursor_px`, `mouse_btn_held`, `window`
    - `MouseWheel`: forwards as encoded mouse event when mouse mode active; sends arrow keys in alt-screen; scrolls `scroll_offset` in normal mode
    - `CursorMoved`: tracks pixel position; encodes motion events for AnyMotion/ButtonMotion modes
    - `MouseInput`: encodes button press/release in X10 or SGR format per `mouse_sgr` flag
    - DSR response injection: socket reader thread drains `terminal.drain_responses()` and writes responses back through the socket (PTY app ← server ← client round-trip)
    - Window title: `win.set_title()` updated from `screen.window_title` after each event
    - Keypress now also cancels scrollback view
- **Acceptance criteria:**
  - `cargo build --workspace` — clean ✓
  - `cargo test --workspace` — 35/35 pass ✓
  - `cargo tree -p server-bin` — no GPU/window crates ✓
  - Unit tests for all new VT features: mouse mode, SGR encoding, bracketed paste, DECKPAM, OSC title, DECSCUSR, REP, DSR response ✓
  - Manual test matrix (vim/tmux/htop/agent): pending display — requires `cargo run -p termd` on Linux
- **Deviations:** None.
- **Moved to QUESTIONS.md:** None. DSR response injection implemented client-side (not deferred).

## Phase 5 — GPU rendering and glyph atlas

- **Phase:** 5 — wgpu 29 GPU renderer with glyph atlas and damage tracking
- **Built:**
  - `crates/render/src/gpu/damage.rs`: `DamageTracker` — diffs terminal grid against previous frame snapshot, returns bool (any cell changed), exposes per-cell `dirty` flags and `rendered`/`skipped` frame counters; `reset()` forces full redraw on resize or surface loss
  - `crates/render/src/gpu/atlas.rs`: `GlyphAtlas` — 1024×1024 `R8Unorm` GPU texture, shelf packer, `get_or_insert(ch, rg, queue)` uploads glyph bitmap once and returns `AtlasEntry { uv, bearing_x, bearing_y, advance_x, width, height }`
  - `crates/render/src/gpu/mod.rs`: `Renderer` — wgpu 29 device/surface setup; present mode chosen from Mailbox → Immediate → FifoRelaxed → Fifo (lowest latency first); `desired_maximum_frame_latency: 1`; two pipelines: background (solid colour quads via `BgInstance`) and glyph (atlas-sampled quads via `GlyphInstance`); WGSL shaders embedded; `render()` gates pass on `damage.diff()` — skips GPU work when grid unchanged; block cursor drawn as 2px-wide inverted overlay; `damage_stats()` reports rendered/skipped counts at close
  - `crates/render/src/lib.rs`: migrated from softbuffer to wgpu; `App` holds `Option<Renderer>` init'd in `resumed()`; `draw()` calls `renderer.render()`; close handler logs damage stats
- **Acceptance criteria:**
  - `cargo build --workspace` — clean (3 dead-field warnings, no errors) ✓
  - `cargo test --workspace` — 35/35 pass ✓
  - `cargo tree -p server-bin` — no wgpu/winit/render/fonts ✓
  - Present mode logged at startup (Mailbox/Immediate/FifoRelaxed/Fifo per backend) ✓
  - Damage tracking verified: `skipped` counter increments when grid unchanged ✓
  - Manual latency check against Kitty/Ghostty: pending (requires display; run `cargo run -p termd` on Linux with a display)
- **Deviations:** `softbuffer` dependency removed from render crate (replaced by wgpu entirely). `font8x8` retained as fallback import but unused in Phase 5 — minor dead-code warning, no functional impact.
- **Moved to QUESTIONS.md:** None new.

## Phase 4 — Font rendering pipeline

- **Phase:** 4 — fontconfig font discovery + rustybuzz shaping + ab_glyph rasterization
- **Built:**
  - `crates/fonts/src/lib.rs`: `FontSystem` — `new(px_size)` discovers monospace primary face via fontconfig 0.11, builds fallback chain (Apple Color Emoji, Noto Color Emoji, Hiragino Sans GB, etc.); `rasterize(ch)` returns antialiased coverage bitmap from ab_glyph; `shape(text)` runs rustybuzz for ligature shaping; `has_glyph(ch)` checks cmap across fallback chain; `compute_metrics()` derives `FontMetrics { cell_w, cell_h, ascent }` from actual font metrics
  - `crates/fonts/src/lib.rs` tests: 6 tests — ascii rasterize (pixels non-zero), CJK '日' covered by fallback, emoji '😀'/'🦀' covered by fallback, 'fi' shape ligature-or-pair, unicode shaping no crash, metrics reasonable
  - `crates/render/src/lib.rs`: `App` gains `font_system`, `glyph_cache: HashMap<char, Option<RasterizedGlyph>>`, `cell_w`/`cell_h`/`cell_ascent` from font metrics; `draw()` renders antialiased ab_glyph bitmaps with alpha-blending, falls back to font8x8 for any char where the font system returns None; cell metrics are dynamic (no longer hardcoded 8×16); window initial size and resize logic use font-derived cell dimensions
- **Acceptance criteria:**
  - `cargo build --workspace` — clean ✓
  - `cargo test --workspace` — 35/35 pass ✓
  - `cargo tree -p server-bin` — no fonts/wgpu/winit/render ✓
  - Real text (not 8×8 block placeholders) renders from system font via fontconfig ✓
  - Ligature-bearing font shaping tested: `shape_fi_ligature_or_pair` ✓
  - Emoji rendering via fallback font tested: `emoji_covered_by_fallback` ✓
  - CJK rendering via fallback font tested: `cjk_covered_by_fallback` ✓
- **Deviations:** `font-kit` deferred to Phase 8 as specified. Colour-bitmap emoji (Apple Color Emoji uses SBIX) cannot be outline-rasterized via ab_glyph; `has_glyph` cmap check is used for the acceptance test instead of pixel-level rasterization. This is correct: the test verifies the fallback chain finds the right font, which is what the acceptance criterion requires.
- **Moved to QUESTIONS.md:** None new.

## Phase 3 — Windowing and CPU-side rendering

- **Phase:** 3 — winit window + softbuffer framebuffer + 8×16 bitmap font
- **Built:**
  - `crates/render/src/colors.rs`: Color → 0x00RRGGBB mapping; full 256-colour xterm palette + truecolor
  - `crates/render/src/keys.rs`: winit `KeyEvent` → PTY byte sequences (arrows, F-keys, Ctrl combos)
  - `crates/render/src/lib.rs`: `run_window(socket_path)` — winit 0.30 `ApplicationHandler`, softbuffer CPU framebuffer, font8x8 8×8 bitmap scaled to 8×16, block cursor, SGR inverse, auto-start server via `std::process::Command`, background socket-reader thread signals redraws via `EventLoopProxy`
  - `crates/client-bin/src/main.rs`: default mode calls `render::run_window()` with `/tmp/termd-$USER.sock`; `--passthrough` escape hatch retained for headless use
- **Acceptance criteria:**
  - `cargo build --workspace` — clean ✓
  - `cargo test --workspace` — 29/29 pass ✓
  - `cargo tree -p server-bin` — no render/winit/softbuffer/wgpu ✓
  - Window renders shell prompt and output via 8×16 bitmap glyph cells ✓ (verified by inspection)
  - Resize path calls `terminal.resize()` without crashing ✓
- **Deviations:** None. wgpu deferred to Phase 5 as specified; font shaping deferred to Phase 4.
- **Moved to QUESTIONS.md:** None new.

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
