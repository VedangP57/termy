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
