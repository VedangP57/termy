# Build phases

Phases are in dependency order — do not start a phase before the previous
one's acceptance criteria are met and checked off. Each phase lists explicit
non-goals; treat scope creep into a later phase's territory as a bug, not
initiative. If a phase feels too slow because of this, raise it as a
question rather than silently pulling later-phase work forward.

## Phase 1 — PTY and shell spawning

**Objective:** prove the absolute floor of the system: open a pseudoterminal,
fork a shell into it, move bytes in both directions.

**In scope:**
- `crates/pty`: wrap `portable-pty` to open a PTY, spawn the user's `$SHELL`
  (fallback `/bin/bash`) attached to it, and expose a simple read/write
  handle.
- A throwaway CLI entry point that spawns a shell and pipes its raw byte
  stream straight to stdout, and forwards stdin straight to the PTY. No
  parsing, no grid, no rendering.
- A first, deliberately naive agent-state-detection stub: watch the raw byte
  stream for the shell's prompt reappearing after a command (a simple
  heuristic, e.g. matching a configured prompt string or detecting a known
  "idle" pattern) and log a state transition to stdout. This does not need
  to be correct or general — it exists to validate, end to end, that the
  PTY layer can feed the agent-detection concept before more infrastructure
  exists. Expect this to be substantially rewritten once Phase 2's grid
  model exists, since the detector should eventually read from structured
  grid/line data, not a raw byte stream.
- Minimal client/server skeleton: the binary accepts a `--server` flag and,
  in that mode, holds the PTY and accepts one local Unix-socket connection
  that just relays bytes (no real protocol yet — that's Phase 8). The point
  of doing this now is to keep the client/server split a structural fact
  from day one rather than retrofitted later, per the hard rule in
  `CLAUDE.md`.

**Out of scope:** any VT/escape-sequence interpretation (raw bytes including
escape codes are simply passed through), any windowing or rendering, any
real wire protocol, resize handling beyond not crashing on it.

**Acceptance criteria:**
- [ ] Running the client binary spawns a real shell; typing commands and
  seeing their output works indistinguishably from a normal terminal when
  viewed through a real terminal emulator wrapping this process's stdout
  (since this phase has no renderer of its own yet).
- [ ] Running with `--server` then attaching a separate client process over
  the Unix socket relays bytes correctly in both directions.
- [ ] The naive state-detection stub logs at least "idle" vs. "busy"
  transitions for simple cases (e.g. running `sleep 3` shows busy, then
  idle).
- [ ] `cargo tree -p server-bin` shows no GPU/windowing crates.

## Phase 2 — VT/ANSI parsing into a grid

**Objective:** turn the raw byte stream into a structured grid of cells.

**In scope:**
- `crates/vt`: parse the byte stream from Phase 1 into a grid of cells
  (character, fg/bg color including 256-color and truecolor, bold/italic/
  underline flags), implemented on the `termwiz`/`wezterm-term` lineage per
  the sub-decision recorded in `02-TECH-STACK.md` (resolve that sub-decision
  before writing code here, not during).
- Scrollback buffer and the alternate screen buffer (what `vim`/`less` use)
  must both work — this is one of the most common sources of subtle bugs in
  terminal emulators, do not treat it as a stretch goal.
- Cursor position/visibility state, including basic cursor movement
  sequences.
- Rewrite the Phase 1 agent-state-detection stub to read from this grid/line
  model instead of raw bytes.

**Out of scope:** any rendering (a debug-only text dump of the grid to
stdout is acceptable for testing, but is not "rendering" in the Phase 3
sense), mouse reporting, bracketed paste, OSC 8 hyperlinks, any Kitty-style
extended protocols (Phase 7).

**Acceptance criteria:**
- [ ] Running real-world programs that are notorious for exercising
  alt-screen and cursor-control edge cases — `vim`, `tmux` (nested, for
  parser stress-testing only, not as an architectural dependency), `htop` —
  through the parser and dumping the resulting grid to a debug view produces
  a recognizable, correct representation of each program's screen.
- [ ] Scrolling back through history after a program that used the
  alt-screen exits returns you to the correct pre-alt-screen scrollback
  content.
- [ ] Unit tests exist for color parsing (16/256/truecolor) and at least the
  cursor-movement and erase-line/erase-display escape sequences, per the
  testing requirements in `05-CONVENTIONS-AND-TESTING.md`.

## Phase 3 — Windowing and basic CPU-side rendering

**Objective:** get the grid from Phase 2 onto an actual screen, with no
performance optimization yet — this phase is about proving the pipeline is
wired correctly end to end, not about speed.

**In scope:**
- `crates/render` skeleton: open a `winit` window, and draw the grid using
  basic CPU-side text rendering (a simple rasterizer or even a placeholder
  monospace block-per-cell rendering) just to see real shell output inside
  an actual GUI window.
- Wire the client binary's normal (non-`--server`) mode to open this window
  and attach to a local or remote server per the Phase 1 skeleton.

**Out of scope:** GPU rendering, font shaping, damage tracking, any
performance tuning. If rendering is visibly slow in this phase, that is
expected and not a bug to fix here.

**Acceptance criteria:**
- [ ] A real shell prompt and basic command output is visible in an actual
  window, not just stdout.
- [ ] Resizing the window doesn't crash and reflows the grid.

## Phase 4 — Font rendering pipeline

**Objective:** real font loading and shaping, still CPU-side rendering.

**In scope:**
- `crates/fonts`: font discovery via `fontconfig` (Linux-only, per
  `02-TECH-STACK.md`), loading a default monospace font plus a fallback
  chain for glyphs missing from it (emoji, CJK, symbols).
- Integrate `rustybuzz` for shaping — ligatures, combining characters,
  complex scripts.

**Out of scope:** GPU glyph atlas (Phase 5), cross-platform font loading
(`font-kit`, deferred per `02-TECH-STACK.md`).

**Acceptance criteria:**
- [ ] Real text (not block placeholders) renders with the correct glyphs,
  including at least one tested case each of: a ligature-bearing font
  rendering correctly, an emoji rendering via fallback, and a CJK character
  rendering via fallback.

## Phase 5 — GPU rendering and glyph atlas

**Objective:** this is the phase that actually delivers "Kitty-tier"
performance — move rendering onto the GPU, with input latency as the
explicit, primary design constraint (see `01-ARCHITECTURE.md` § 5).

**In scope:**
- Move rendering onto `wgpu`: rasterize each glyph once into a texture
  atlas, render frames as textured quads.
- Damage tracking: only redraw changed cell regions.
- Render-on-input scheduling rather than a fixed frame tick.
- Select and configure the lowest-latency present mode `wgpu` exposes for
  the backend in use; do not default to a smooth/buffered present mode
  without explicitly recording why if changed later.

**Out of scope:** any Kitty-tier *protocol* features (inline images, sync
output, kitty keyboard protocol — that's Phase 7); this phase is about the
rendering pipeline's speed, not feature parity.

**Acceptance criteria:**
- [ ] Typing and scrolling are subjectively checked against the manual
  latency test matrix in `05-CONVENTIONS-AND-TESTING.md` and do not feel
  laggy compared to Kitty/Ghostty/Alacritty run side by side on the same
  machine.
- [ ] Damage tracking is verified to actually skip redrawing unchanged
  regions (not just present, but observably reducing redraw work — e.g. via
  a debug overlay or counter), not merely implemented and unused.

## Phase 6 — Terminal feature completeness

**Objective:** the long tail of real-world correctness. Budget more time
than feels reasonable — this is where most terminal projects actually spend
their time.

**In scope:** mouse reporting, bracketed paste, full 256-color/truecolor
coverage if any gaps remain, OSC 8 hyperlinks, scrollback search, robust
resize handling, and fixing whatever real-world program incompatibilities
surface from broader manual testing (see test matrix in
`05-CONVENTIONS-AND-TESTING.md`).

**Out of scope:** Kitty-tier extended protocols (Phase 7).

**Acceptance criteria:**
- [ ] The full manual test matrix in `05-CONVENTIONS-AND-TESTING.md` passes.
- [ ] No known correctness regression list remains open for `vim`, `tmux`,
  `htop`, and a Claude Code/Codex-style CLI agent run directly in the
  terminal (this last one specifically exercises real-world output patterns
  the agent-state detector needs to handle).

## Phase 7 — Advanced protocols (optional, add only if needed)

**Objective:** parity with what real-world tools increasingly expect, not
pursued for its own sake.

**In scope (add incrementally, only as a real need surfaces):** the Kitty
inline image protocol, synchronized output (used by e.g. Neovim for
tear-free rendering), the Kitty keyboard protocol for disambiguating keys
legacy terminals can't.

**Acceptance criteria:** defined per feature when undertaken; do not block
Phase 8 on this phase being "complete," since it has no fixed scope.

## Phase 8 — Client-server completeness, agent protocol, and polish

**Objective:** fully realize the client-server and agent-API vision that
was structurally present since Phase 1, plus platform/UX polish.

**In scope:**
- Full wire protocol per `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md`: session
  persistence across client disconnect, SSH remote attach, multiple
  workspaces/tabs/panes per server, detach/reattach.
- The agent-state-detection socket API: expose pane state
  (blocked/working/done/idle) and basic control (create pane, send input,
  read recent output) to external processes.
- Tabs/splits at the client UI level, clipboard, IME input, config file,
  themes.
- Cross-platform groundwork (macOS/Windows) if it's still a goal at this
  point — this is the first phase where `font-kit` and platform `cfg`
  blocks are allowed in, per `02-TECH-STACK.md`.

**Acceptance criteria:**
- [ ] Detaching a client (closing the window) and reattaching later
  restores all panes' state and scrollback.
- [ ] Remote attach over SSH to a server running on another host works.
- [ ] An external script can connect to the socket API, list panes, read a
  pane's current state, and inject input into a pane.
