# Conventions and testing

Engineering standards for this codebase. These exist so that code written in
different sessions/phases is consistent enough to review and maintain as one
project, not a patchwork of whatever style seemed reasonable in the moment.

## 1. Error handling

- No `unwrap()`/`expect()`/`panic!()` in any code on the PTY read/write path,
  the VT parsing path, or the render loop. A malformed escape sequence, an
  unexpected EOF, or a transient I/O error on these paths must be handled
  (logged, recovered from, or surfaced as a typed error) — these paths
  process untrusted/unpredictable input (real-world programs emit weird
  escape sequences; PTYs can close unexpectedly) and crashing the whole
  terminal because of one bad byte is a regression, not an edge case.
- `unwrap()` is acceptable in test code and in one-off debug/throwaway
  binaries explicitly marked as such, never in library crates under
  `crates/`.
- Use a typed error enum per crate (e.g. via `thiserror`) rather than a
  single project-wide error type; do not let internal error types leak
  across crate boundaries without being wrapped/converted at the boundary.

## 2. `unsafe` code policy

- `unsafe` is allowed only where a chosen dependency's API requires it
  (e.g. certain `portable-pty`/raw-syscall edge cases) or where a measured
  performance need justifies it (e.g. a hot loop in the glyph atlas or VT
  parser, only after profiling shows a real bottleneck — not preemptively).
- Every `unsafe` block must have a comment immediately above it explaining
  the specific invariant being relied on and why it holds. "this is faster"
  with no invariant explanation is not sufficient justification to merge.
- Prefer reaching for a safe abstraction in a well-maintained crate over
  hand-rolling `unsafe` — this is part of why `portable-pty`, `wgpu`, and
  `winit` were chosen over lower-level alternatives in `02-TECH-STACK.md`.

## 3. Module and crate layout

- Follow the workspace layout in `02-TECH-STACK.md` exactly; do not
  introduce a new top-level crate without updating that document.
- Enforce the client/server dependency boundary mechanically where
  possible — e.g. a CI check running `cargo tree -p server-bin` and failing
  if `wgpu` or `winit` appear, rather than relying on review alone to catch
  a layering violation.
- Within `crates/vt`, keep the byte-stream parser and the grid/scrollback
  model as separately testable units (even if they end up in the same
  crate) so parser-only unit tests don't require constructing a full grid.

## 4. Performance discipline

- Anything claimed to be a latency- or throughput-relevant change should be
  measured, not assumed — at minimum, a before/after note in the PR
  description describing what was measured and how, per the targets in
  `01-ARCHITECTURE.md` § 5.
- Do not add caching, buffering, or batching to the PTY read/write or
  render-input path "for efficiency" without checking it doesn't regress
  input latency specifically — throughput-motivated optimizations are a
  common way to accidentally add a frame or two of input delay.

## 5. Required test types

### Unit tests (required from Phase 2 onward)

- VT parser: one test per escape-sequence category actually implemented
  (cursor movement, color modes including 16/256/truecolor, erase
  line/display, alt-screen enter/exit, at minimum).
- Grid model: scrollback behavior, including the specific case of
  alt-screen exit correctly restoring pre-alt-screen scrollback content.
- Agent-state detector (from Phase 2's real implementation onward): one
  test per state transition the heuristics claim to detect.

### Golden-file tests (required from Phase 2 onward)

- Capture real byte-stream output from running real programs (`vim`,
  `tmux`, `htop`, a long-running build command, a CLI coding agent) once,
  store it as a fixture, and assert the parsed grid matches an expected
  snapshot. This is how terminal-emulator correctness regressions get
  caught in practice — add a new fixture whenever a real-world program
  surfaces a parsing bug, so the bug can't silently come back.

### Manual test matrix (required before any phase's acceptance criteria
referencing it can be marked done — see `03-BUILD-PHASES.md`)

Run each of the following directly in the terminal being built (not in a
nested/wrapping terminal) and confirm it behaves correctly and feels
responsive:

- `vim` — alt-screen, syntax-highlighted color rendering, scrolling.
- `tmux` (nested) — stresses the parser with another program that itself
  manages a grid/alt-screen; this is a parser stress test, not an
  architectural endorsement of building a TUI-on-top-of-a-terminal (see the
  rejected Option A in `01-ARCHITECTURE.md`).
- `htop` — frequent partial-screen redraws, color, resizing while running.
- A long-running build or test command — throughput under heavy output.
- A CLI coding agent (Claude Code, or equivalent) — exercises the
  agent-state-detection patterns directly, since this is the project's
  actual differentiator; treat any misclassification found here as a real
  bug per `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 7.2.
- Side-by-side input-latency comparison against Kitty/Ghostty/Alacritty on
  the same machine — typing and scrolling should not feel laggier; this is
  a subjective check but should be done deliberately, not skipped because
  it's not a numeric assertion.

### Integration tests for the client-server split (required from Phase 8)

- Detach/reattach preserves pane state and scrollback (per
  `03-BUILD-PHASES.md` Phase 8 acceptance criteria).
- A test (or documented manual procedure) exercising the SSH remote-attach
  path end to end, not just the local-socket path.

## 6. Documentation hygiene

- When a decision recorded in `01-ARCHITECTURE.md`, `02-TECH-STACK.md`, or
  `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` changes for a real, deliberate
  reason, update that document in the same change — do not let the docs
  drift from what the code actually does. A stale "rejected alternative"
  section that the code has actually adopted is worse than no documentation
  at all, since it actively misleads.
- New open decisions discovered during implementation get added to the
  relevant document's "open decisions" section (see e.g.
  `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 9) rather than resolved
  silently inline in code with no record of the choice having been made.
