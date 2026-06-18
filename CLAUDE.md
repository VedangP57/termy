# Project memory — read this first

This file is the entry point. It is intentionally short. Every deep technical
decision lives in `docs/` and is loaded via the imports below — read the
imported file fully before writing code in that area. Do not skip the import
and guess; the rejected-alternatives sections exist specifically to stop you
from re-proposing something that was already decided against.

## What this project is

A from-scratch, GPU-accelerated terminal emulator, written entirely in Rust,
targeting input-latency and rendering performance in the same tier as Kitty
and Ghostty. On top of the renderer sits a client-server multiplexing layer
(local sessions + SSH remote attach + detach/reattach persistence, in the
spirit of tmux/Herdr) with an agent-state-detection layer as the product's
actual differentiator (parsing PTY output to classify panes as
blocked/working/done/idle, exposed over a Unix socket API).

This is NOT a wrapper around an existing terminal (no libghostty FFI, no
forking Kitty, no TUI-on-top-of-an-existing-terminal architecture). The
renderer is written by us, end to end. See `docs/01-ARCHITECTURE.md` for the
full reasoning — do not re-litigate this in code review or design docs.

## Hard rules (non-negotiable, do not ask to revisit)

- Language: Rust only. No Zig, no C/C++, no Electron, anywhere in this repo.
- VT parsing / grid model: written independently in `crates/vt`, using
  `wezterm-term`/`termwiz` only as a *reference* to read, never as a Cargo
  dependency. **Never** add `alacritty_terminal`, `termwiz`, or
  `wezterm-term` to any `Cargo.toml` in this workspace — all three were
  explicitly considered and rejected as dependencies for this layer.
- Architecture: full custom renderer (PTY → VT parser → grid → wgpu). Never
  implement this as a `ratatui`-style TUI layer riding on top of an existing
  terminal — that is a different, rejected project.
- Every terminal-facing binary supports both client and server mode from a
  single binary (`--server`, default client). Never split this into two
  separate crates/binaries.
- Optimize for input latency over throughput when the two trade off. See
  `docs/01-ARCHITECTURE.md` § Performance Targets before touching the render
  loop or present-mode config.
- Platform target for all early phases: Linux only. Do not add
  macOS/Windows-specific code, `font-kit`, or cross-platform conditional
  compilation until `docs/03-BUILD-PHASES.md` explicitly says Phase 8+.

## Workspace layout

```
crates/
  pty/          # portable-pty wrapper, shell spawning
  vt/           # VT/ANSI parser + grid/scrollback (written by us, no termwiz dep)
  render/       # winit + wgpu, glyph atlas, damage tracking (client-only)
  fonts/        # fontconfig + rustybuzz shaping (client-only)
  protocol/     # wire format types shared by client/server
  agentd/       # agent-state-detection heuristics + socket API
  server-bin/   # --server entry point
  client-bin/   # default (client) entry point
```

Strict layering rule: `cargo tree -p server-bin` must never show `wgpu` or
`winit`. If it does, that is a layering bug — fix before committing.

## Dependency rules

- Never add `alacritty_terminal`, `termwiz`, or `wezterm-term` to any Cargo.toml.
- Never add `ratatui` or any TUI framework.
- Never add `font-kit` before Phase 8.
- Never add `vte` — explicitly considered and rejected (see `docs/02-TECH-STACK.md`).
- Before adding any new dependency: read `docs/02-TECH-STACK.md` for that
  layer's rejected alternatives — the "obvious" choice was often already weighed.

## Coding standards (always apply — load docs/05-CONVENTIONS-AND-TESTING.md for full detail)

**Error handling:**
- No `unwrap()`/`expect()`/`panic!()` in any code on the PTY read/write path,
  VT parsing path, or render loop. These paths process untrusted input —
  crashing on a bad byte sequence is a regression, not an edge case.
- `unwrap()` is acceptable only in test code and explicitly-marked throwaway binaries.
- Typed error enum per crate via `thiserror`. Do not let internal error types
  leak across crate boundaries without wrapping.

**Unsafe:**
- `unsafe` only where a chosen dependency's API requires it, or where profiling
  has proven a real bottleneck (not preemptively).
- Every `unsafe` block must have a comment explaining the specific invariant
  being relied on. "this is faster" alone is not sufficient.

**Tests (required from Phase 2 onward):**
- Unit tests: one per escape-sequence category implemented, per state
  transition the agent-state detector claims to detect.
- Golden-file tests: capture real program output (`vim`, `tmux`, `htop`) as
  a fixture, assert the parsed grid matches an expected snapshot.
- Add a new fixture whenever a real-world program surfaces a parsing bug.

## Git workflow (follow exactly — never deviate)

- After every meaningful set of changes, stage the specific files changed and
  commit: `git add <files>` then `git commit -m "..."`. Write a real commit
  message — what changed and why, not "update files."
- Commit granularity: one logical change per commit. Do not batch unrelated
  changes.
- **Never `git push`** — the project owner pushes manually.
- Never use `git add -A` or `git add .` — stage specific files by name to
  avoid accidentally including secrets or build artifacts.

## Agent-state model (the product differentiator — treat with care)

Four states per pane, exposed over the socket API:
- **idle**: shell prompt is showing, nothing running.
- **working**: foreground process running, output activity recent.
- **blocked**: foreground process waiting on user input (tool confirmation,
  "Press Enter" pattern, etc).
- **done**: foreground process exited, not yet acknowledged.

Detection must operate on parsed grid/line data (Phase 2+), not raw bytes.
False positives/negatives are real bugs, not inherent fuzziness — the whole
product depends on this being trustworthy enough to "close the laptop" on.

## Always load before working in these areas

- @docs/01-ARCHITECTURE.md — full system design, data flow, explicitly
  rejected alternatives and why. Load before touching crate boundaries,
  the render loop, or anything described as "the engine."
- @docs/02-TECH-STACK.md — exact crates, versions, and rationale per layer.
  Load before adding any new dependency to Cargo.toml.
- @docs/03-BUILD-PHASES.md — the phase plan, in dependency order, with
  acceptance criteria per phase. Re-read the section for the current phase
  before writing any code for it — even if you already read it earlier in
  this session. Do not rely on memory of a prior read.
- @docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md — IPC design, wire protocol,
  session model, agent state-detection heuristics. Load before writing
  anything under `crates/protocol/` or `crates/agentd/`.
- @docs/05-CONVENTIONS-AND-TESTING.md — error handling policy, `unsafe` code
  policy, module layout rules, required test types. Load before opening a PR
  or writing tests.
- @docs/06-GLOSSARY.md — term definitions (PTY, damage tracking, alt-screen,
  etc.). Load if a term in another doc is unfamiliar rather than guessing.

## Build / run

```
cargo build --workspace
cargo test --workspace
cargo tree -p server-bin          # verify no GPU/windowing crates in server
cargo run -p termd -- --server   # start server mode
cargo run -p termd               # start client mode, attaches to local server
```

## Current state

Phase 1 not yet started. Do not write Phase 2+ code (VT parsing, windowing,
GPU rendering) until Phase 1's acceptance criteria in
`docs/03-BUILD-PHASES.md` are met and checked off.

## When in doubt — QUESTIONS.md protocol

Do not silently resolve open decisions or deviate from hard rules. Instead:

1. Write the question or conflict clearly to `QUESTIONS.md` at the repo root.
2. Move on to other work that doesn't depend on that decision.
3. Never guess at an open decision and never silently revert a rejected alternative.

This applies to everything listed under an "Open decisions" heading in any doc
(e.g. `docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 9) and to any new
conflicts or ambiguities discovered during implementation.

QUESTIONS.md is how visibility is maintained when running unattended — the
project owner reads it periodically instead of watching the terminal.

## Stuck-loop protocol

If you fail the same test or hit the same error three consecutive times on one task:

1. **Stop immediately.** Do not make increasingly large changes trying to force it to pass.
2. **Write to QUESTIONS.md:** what the task is, what you tried (all three attempts, briefly), and why it's not working as best you can tell.
3. **Move to a different, independent task** within the current phase. If nothing in the current phase is independent of the blocker, move to the next phase's prep work (reading docs, scaffolding structure, etc.).
4. **Do not return to the blocked task** until the relevant QUESTIONS.md entry has been answered by the project owner.

Three strikes is three. Not four. Not "one more try with a bigger change."
