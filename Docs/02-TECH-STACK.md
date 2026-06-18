# Technology stack

Exact crate choices per layer, why, and what was explicitly rejected at each
layer. If a new dependency is being considered for a layer listed here, the
rejected alternative for that layer was already considered — do not silently
swap it back in; flag the conflict and ask instead.

## Layer-by-layer

### PTY and shell spawning

- **Chosen: `portable-pty`.** Used by WezTerm; wraps the platform PTY
  syscalls (`posix_openpt`/`forkpty` on Unix) behind one API.
- **Rejected: hand-rolled `nix` + raw `fork`/`exec`/`ioctl`.** This is what
  `alacritty_terminal` does internally. Gives more control, costs more time
  debugging platform-specific syscall edge cases for no benefit at this
  project's current stage. Revisit only if `portable-pty` is found to have a
  hard limitation that blocks a specific feature — and document what that
  limitation was if so.

### VT/escape-sequence parsing and grid model

- **Chosen: the `wezterm-term`/`termwiz` lineage.**
- **Explicitly rejected: `alacritty_terminal`.** This is the most important
  rejection in the whole stack to remember — it would be the "obvious" easy
  choice (it's the most commonly reused terminal-grid crate in the Rust
  ecosystem) and it will keep looking attractive mid-project. It was
  rejected specifically because the project owner wants to own and
  understand the VT parser/grid implementation rather than depend on an
  external one. Do not add it as a dependency, including transitively, and
  do not implement a "fallback" path that uses it.
- **Resolved: reference only, not a dependency.** `termwiz`/`wezterm-term`
  source is read as a reference while writing an independent parser and
  grid module in `crates/vt`; neither crate is added to any `Cargo.toml` in
  this workspace. Copying code verbatim is not required to avoid (WezTerm
  is MIT-licensed — this is a licensing non-issue), but the implementation
  must be genuinely independent, because the whole point of rejecting
  `alacritty_terminal` was to own and understand this layer rather than
  depend on someone else's. Taking `termwiz`/`wezterm-term` as a direct
  dependency instead would just repeat that exact tradeoff under a
  different crate name — do not revisit this as "faster to ship" without a
  new explicit decision from the project owner, since shipping speed was
  already weighed against and rejected in favor of ownership/learning when
  `alacritty_terminal` was turned down.
- If `cargo tree` in this workspace ever shows `termwiz` or `wezterm-term`
  as a dependency (even transitively), that's a violation of this decision,
  not an acceptable shortcut — flag it rather than merging it.
- **Rejected (parser-only option, for completeness): `vte`.** This is just
  the byte-stream-to-callback parser, with no grid/scrollback/alt-screen
  model on top — you'd build that state machine yourself. Not chosen because
  it doesn't change the core question (own vs. borrow the grid model) and
  adds pure extra work either way; noted here so it isn't "discovered" later
  and treated as a new option.

### Windowing

- **Chosen: `winit`.** De facto standard cross-platform windowing crate in
  the Rust GPU-app ecosystem; used by Alacritty and effectively the whole
  `wgpu` ecosystem. Handles window creation, input events, resize across
  Linux X11/Wayland (macOS/Windows deferred — see `01-ARCHITECTURE.md` § 6).
- No serious alternative was evaluated for this layer; it is the standard
  choice and not a contested decision.

### GPU rendering

- **Chosen: `wgpu`.** Safe Rust abstraction over Vulkan/Metal/DX12/OpenGL —
  one rendering backend, runs everywhere. This is the modern choice; newer
  Rust graphics projects converge on it.
- **Rejected: raw OpenGL via `glow`/similar (Alacritty's actual approach).**
  Alacritty uses raw OpenGL because it predates `wgpu`'s maturity. Not
  chosen here because `wgpu` gives better safety guarantees and a clearer
  path to other backends without a rewrite, at no real cost for this use
  case.
- Required behavior, not just a library choice: glyphs are rasterized once
  into a texture atlas (this is what makes Kitty's rendering cost scale with
  unique glyphs, not character count) and frames are drawn as textured
  quads, not re-rasterized per frame. Damage tracking (redraw only changed
  regions) is required in Phase 5, not optional.

### Fonts

- **Chosen (Phase 4, Linux-only): `fontconfig` used directly.** Simpler than
  going through an abstraction layer when only targeting Linux.
- **Deferred, not rejected: `font-kit`.** This is the right choice once
  cross-platform (macOS/Windows) support becomes an actual near-term goal
  (Phase 8+) — wraps platform font APis (fontconfig/Core Text/DirectWrite)
  behind one interface. Do not add it before then; it's an unnecessary
  abstraction layer while still Linux-only.

### Text shaping

- **Chosen, added in Phase 4 once basic Latin rendering works: `rustybuzz`.**
  Pure-Rust port of HarfBuzz. Required for ligatures, complex scripts, and
  combining characters — not optional for anything beyond plain ASCII, but
  intentionally stubbed out until basic glyph rendering is proven end to
  end, to avoid debugging two new systems (GPU pipeline + shaping) at once.

### IPC / client-server transport

- **Chosen: Unix domain sockets for local client↔server, with the same
  binary run on a remote host (via SSH) and a local client attaching through
  that SSH session** — i.e., the client speaks the protocol over the
  connection it has (local socket, or stdio piped through `ssh host termd
  --server`), not by trying to expose the Unix socket itself over the
  network. See `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` for the actual wire
  format.
- **Rejected: exposing a TCP/network socket directly.** Avoids having to
  build and audit any network-facing auth story; SSH already solves
  transport security, so don't reimplement it.

### Wire protocol / serialization

- **Chosen: a length-prefixed binary frame format with `serde` +
  a compact binary codec (e.g. `bincode` or equivalent) for the control
  protocol; raw bytes (not re-encoded) for the actual PTY data stream
  between client and server.** Decide the exact serialization crate at
  Phase 8 implementation time, but do not use a text/JSON protocol for the
  hot-path PTY data stream — JSON-encoding terminal output is unnecessary
  overhead in the latency-sensitive path. JSON is acceptable for the
  one-off socket API used by external agents (see
  `04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md`), since that path is not
  per-keystroke latency sensitive.

## Notes on Kitty/Ghostty extensibility (for context, not a dependency)

Recorded here so this isn't "re-researched" mid-project and used to justify
reopening Section 2 of `01-ARCHITECTURE.md`:

- Kitty has no embeddable rendering core. Its extension model ("kittens") is
  Python programs that get access to *internal* kitty APIs, which are
  explicitly undocumented and unstable; the *supported* extension surface is
  a separate Remote Control API for scripting a running instance, not a
  library you link against to build a new terminal. There is no "libkitty."
- Ghostty's core (`libghostty`) is real and under active development
  (`libghostty-vt`, a zero-dependency Zig module for parsing/state, has
  shipped), and real products embed it today (cmux does, via Swift/AppKit).
  It remains Zig-first; a stable C API is in progress but not yet shipped at
  time of writing. This is why Option C in `01-ARCHITECTURE.md` is closed —
  not because the pattern doesn't work, but because of the Rust-only
  constraint plus current API immaturity for non-Zig consumers.

## Workspace layout (Cargo workspace, multiple crates)

```
/
├── Cargo.toml                  # workspace manifest
├── crates/
│   ├── pty/                    # portable-pty wrapper, shell spawning
│   ├── vt/                     # VT/ANSI parser + grid/scrollback model
│   ├── render/                 # winit + wgpu, glyph atlas, damage tracking
│   │                           #   (client-only; never depended on by server)
│   ├── fonts/                  # fontconfig loading, rustybuzz shaping
│   │                           #   (client-only)
│   ├── protocol/                # wire format types shared by client/server
│   ├── agentd/                  # agent-state-detection heuristics + socket API
│   ├── server-bin/              # the `--server` binary entry point
│   └── client-bin/              # the default (client) binary entry point
└── docs/                        # this documentation set
```

Strict rule: `crates/render` and `crates/fonts` must never appear in
`server-bin`'s dependency tree. If `cargo tree -p server-bin` ever shows
`wgpu` or `winit`, that is a layering bug, not a style nit — fix it before
merging.
