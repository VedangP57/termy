# Architecture

This document is the canonical description of what this system is, how data
flows through it, and — just as important — what it explicitly is not. The
"rejected alternatives" sections are not historical trivia; they record real
options that were seriously considered and ruled out during planning. Do not
reintroduce them without a new, explicit decision from the project owner.

## 1. Vision

A terminal emulator, written entirely in Rust, that:

1. Renders terminal output via the GPU at the same performance and input
   latency tier as Kitty and Ghostty.
2. Is structured from day one as a client-server system, so sessions persist
   across disconnects and can be attached to locally or over SSH, the way
   tmux and Herdr work.
3. Adds a layer that classifies what each pane is doing — blocked, working,
   done, idle — by observing PTY output, and exposes that state (and control
   over panes) through a socket API that coding agents (Claude Code, Codex,
   etc.) can drive. This is the actual differentiator; the renderer is the
   floor you have to build to get there, not the product itself.

## 2. Why a custom renderer, not a wrapper

This was the single most contested decision in planning and is finalized.
Three architectures were evaluated:

**Option A — TUI multiplexer layer on top of an existing terminal**
(the Herdr model). A program draws panes/tabs using a terminal UI framework
(`ratatui` in Rust) inside whatever GPU terminal the user already has open.
You never touch rendering, fonts, or GPU code; you only manage PTYs and draw
box-drawing characters. Fast to build (weeks), but the renderer is not yours
— "fast like Kitty" is borrowed, not engineered.

**Option B — Build the GPU-rendered terminal from scratch.**
PTY handling, VT/escape-sequence parsing, a grid model, font shaping, and a
GPU rendering pipeline, all written by this project. Months of work, much of
it in well-trodden territory (PTY syscalls, VT parsing, GPU text rendering
all have mature crates), but the speed and correctness are actually owned by
this codebase rather than delegated to someone else's binary.

**Option C — Embed an existing engine via FFI** (the cmux model: a thin
Swift/AppKit shell wrapping `libghostty` for rendering). This is a real,
shipped pattern — cmux is exactly this — but `libghostty`'s public API
surface is Zig-first and not yet stabilized for C consumers, which conflicts
with the hard "Rust only" constraint. This option is closed, not because the
pattern is bad, but because of the language constraint.

**Decision: Option B.** Rationale, for the record: the project owner
explicitly wants to write the VT parser and grid model rather than depend on
someone else's (this also ruled out `alacritty_terminal` as a dependency —
see `02-TECH-STACK.md`). Given that, Option A stops being relevant (it
specifically avoids writing a renderer at all) and Option C is closed by the
language constraint. Do not propose Option A or C as a "faster path" later —
this has already been weighed and rejected with full knowledge of the time
cost.

## 3. High-level data flow

```
┌─────────────┐     bytes      ┌──────────────────┐
│  child shell │ ─────────────▶│   PTY (master)    │
│ (zsh/bash/…) │◀───────────── │                    │
└─────────────┘     bytes      └────────┬───────────┘
                                         │ read loop
                                         ▼
                              ┌───────────────────────┐
                              │   VT/ANSI parser        │
                              │ (termwiz-lineage)        │
                              └────────┬─────────────────┘
                                         │ parsed actions
                                         ▼
                              ┌───────────────────────┐
                              │   Grid / screen model     │
                              │ (cells, scrollback,        │
                              │  alt-screen, cursor)        │
                              └───┬─────────────────┬──────┘
                                  │                   │
                                  ▼                   ▼
                  ┌─────────────────────┐  ┌───────────────────────┐
                  │  Renderer (wgpu)      │  │ Agent state detector    │
                  │  glyph atlas, damage   │  │ (heuristics over the     │
                  │  tracking, present      │  │  same parsed stream)      │
                  └─────────────────────┘  └────────────┬─────────────┘
                                                          ▼
                                              ┌───────────────────────┐
                                              │  Socket API server      │
                                              │  (workspace/pane state,  │
                                              │   agent control)          │
                                              └───────────────────────┘
```

Both the renderer and the agent-state detector read from the same grid/parsed
stream — they are not two separate parsing passes. The renderer cares about
"what cells changed since the last frame" (damage tracking); the agent
detector cares about "what does the new output mean" (prompt returned, error
pattern, known agent-CLI marker, etc). Keep these as two consumers of one
parsed model, not two parsers.

## 4. Process model: client and server

Single binary, two modes, selected by an argument (not by which file you
compiled):

- **Server mode**: owns the actual PTYs and the grid/scrollback state for
  every pane in every workspace it manages. Has no rendering code in it at
  all — it is headless. Listens on a Unix domain socket (local) and can be
  reached over SSH by running the same binary on the remote host and having
  a local client attach to it through the SSH connection (not by tunneling
  the socket — see `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md`).
- **Client mode**: owns the window, the GPU renderer, font rendering, and
  input handling. Holds no PTYs itself — it sends input to the server and
  receives a stream of grid updates/damage regions to render. Detaching the
  client (closing the window, losing the SSH connection) does not kill the
  server or any pane's process; reattaching gets you back to where you were.

This means the renderer (Section 3) only ever runs inside the client
process, and the server process must be buildable and runnable without
`wgpu`/`winit` linked in at all — those are client-only dependencies. Keep
the crate boundaries (see `02-TECH-STACK.md` § Workspace Layout) strict about
this; do not let GPU crates leak into the server binary's dependency tree.

## 5. Performance targets

The qualitative goal stated by the project owner was specifically "Kitty-tier
input latency," not raw throughput. These are different problems and the
codebase should be optimized for the one that was actually asked for:

- **Input latency** (keypress → pixel change on screen): minimize this above
  all else. Concretely: render-on-input rather than waiting for a fixed
  frame tick; use the lowest-latency GPU present mode `wgpu` exposes for the
  current backend rather than defaulting to a smooth/buffered mode; avoid
  unnecessary buffering when reading the PTY's echoed bytes back.
- **Throughput** (how fast a huge burst of output, e.g. `cat` of a large
  file, gets to the screen): matters, but is a secondary concern. Damage
  tracking (only re-rendering changed cells) is the lever here — it does not
  meaningfully help latency, so do not justify a latency-relevant
  architecture decision by pointing at damage-tracking benefits.
- Do not chase a numeric "X% faster than Kitty" target — there is no stable
  ground truth for this across hardware and workloads, and Kitty/Ghostty
  benchmarks against each other already show small, workload-dependent
  differences. The target is "does not feel laggy under real use," validated
  by the manual test matrix in `05-CONVENTIONS-AND-TESTING.md`, not a
  synthetic benchmark score.

## 6. Explicitly out of scope (do not build these without a new decision)

- Any TUI-multiplexer mode that runs inside an existing terminal instead of
  rendering its own window (this is Option A above — rejected).
- Any FFI binding to `libghostty`, Kitty's internals, or any other external
  terminal's rendering core (this is Option C above — rejected, and Kitty
  has no embeddable core to bind to regardless — see
  `02-TECH-STACK.md` § Notes on Kitty/Ghostty extensibility).
- Cross-platform support (macOS, Windows) before Phase 8. Do not add
  `font-kit`, platform `cfg` blocks, or Windows PTY handling before then.
- A garbage-collected or scripting-layer config/extension system (e.g. a
  Lua or Python scripting layer like WezTerm's or Kitty's "kittens"). Not
  ruled out forever, but out of scope for the phases currently planned —
  see `03-BUILD-PHASES.md`.
