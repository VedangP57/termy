# Client-server architecture and agent protocol

This document covers the two things that make this project more than "a
terminal emulator": the client-server session model, and the agent-state
detection layer that's the actual product differentiator. Read this fully
before writing anything under `crates/protocol/` or `crates/agentd/`.

## 1. Why client-server at all

Stated goal: sessions should survive disconnects and be attachable from
anywhere, the way tmux and Herdr work — not just "a fast terminal," but a
multiplexer with agent awareness that the project owner can leave running
on a machine, close the laptop, and come back to. This is structurally
present from Phase 1 (see `03-BUILD-PHASES.md`), not retrofitted later.

## 2. Process roles

- **Server**: headless. Owns every PTY, every pane's grid/scrollback state,
  and the agent-state detector for each pane. Never links `wgpu`/`winit`
  (see the layering rule in `02-TECH-STACK.md`). Runs as a long-lived
  background process, typically started once per machine (or once per
  remote host you SSH into) and left running.
- **Client**: owns the window, renderer, fonts, and input handling. Sends
  user input to the server, receives grid updates/damage regions back.
  Holds no PTYs. Multiple clients may exist over the server's lifetime
  (you can close the window and reopen it later — that's a new client
  process attaching to the same still-running server).

## 3. Local transport

A Unix domain socket, one per server instance, at a well-known path (e.g.
under `$XDG_RUNTIME_DIR` or `~/.local/state/<project>/`). The client
connects to this socket directly for local use. Do not expose a TCP socket
for this (see the rejected alternative in `02-TECH-STACK.md`).

## 4. Remote transport (SSH)

The same binary runs on the remote host in `--server` mode. The local
client does **not** try to forward or tunnel the remote Unix socket back
to the local machine. Instead: the local client spawns `ssh <host>
<binary> --server --attach-stdio` (or equivalent), and speaks the wire
protocol over that SSH session's stdin/stdout, exactly as it would over the
local socket. This means the wire protocol layer must be transport-agnostic
— it should not assume "I am reading from a Unix socket file descriptor,"
it should accept any duplex byte stream (a trait/abstraction over
`AsyncRead + AsyncWrite` or equivalent), with the local-socket and
SSH-stdio cases both implementing it.

## 5. Session model

- A **server** instance manages one or more **workspaces**.
- A **workspace** is a named collection of **tabs**.
- A **tab** contains one or more **panes** (via splits).
- A **pane** owns exactly one PTY and one grid/scrollback state.

Detaching a client does not affect any of the above — they live entirely in
the server process. Reattaching (a new client connecting) must reconstruct
the client-side render state (grid contents, scrollback as far as the
client wants to keep in memory) from the server's authoritative state, not
assume continuity from a previous client session.

## 6. Wire protocol

Two distinct sub-protocols, deliberately different in design because they
have different latency/structure needs (see `02-TECH-STACK.md` § Wire
protocol):

### 6.1 Control protocol (session/workspace/pane management)

Structured, length-prefixed binary frames carrying typed messages (create
workspace, create pane, split pane, resize, list panes, query agent state,
attach to pane, detach, etc). Use `serde` with a compact binary codec for
this — it is not on the per-keystroke hot path, so the main requirement is
correctness and ease of evolving the schema, not raw speed.

### 6.2 PTY data stream (the hot path)

Raw bytes, not re-encoded into a structured message per keystroke/output
chunk — wrap them in the thinnest possible framing (e.g. a pane ID and a
length, then raw bytes) rather than serializing through a generic
`serde`-based message for every chunk of terminal output. This is the path
input latency is measured on; do not add unnecessary allocation or copying
here. Damage-region updates from server to client (if the server, rather
than the client, ends up owning damage computation — decide this explicitly
during Phase 5/8 implementation and record the decision here once made) are
part of this hot path too.

## 7. Agent-state detection

This is the actual differentiator (see `01-ARCHITECTURE.md` § 1.3) — treat
it with real design attention, not as an afterthought bolted onto whatever
parsing already exists.

### 7.1 States

Four states per pane, matching the prior art this project is positioned
against (Herdr-style):

- **idle**: shell prompt is showing, nothing is running.
- **working**: a foreground process is running and has produced output
  recently.
- **blocked**: a foreground process appears to be waiting on user input
  (e.g. a prompt-like pattern that isn't the shell's own prompt, or a known
  "Press Enter to continue"/agent-tool confirmation pattern).
- **done**: a foreground process that was running has exited, but the
  client/user hasn't yet "acknowledged" it (the exact acknowledgement
  mechanism — does viewing the pane clear this, does it require an explicit
  action — is an open decision, see § 9).

### 7.2 Detection approach, by phase

- **Phase 1 (naive stub):** simple string/pattern matching directly on the
  raw byte stream — does the shell's configured prompt string reappear.
  Acceptable to be wrong often; exists only to validate plumbing.
- **Phase 2 onward (real implementation):** operate on the parsed grid/line
  model from `crates/vt`, not raw bytes. Concretely: track the last rendered
  line(s) and look for (a) the shell's own prompt pattern reappearing →
  idle, (b) the cursor sitting at a fresh prompt with no foreground child
  process → idle (cross-check against actual process state if available,
  not pattern-matching alone, since prompt strings can appear in output that
  isn't actually a prompt), (c) recent output activity with a live
  foreground child process → working, (d) known confirmation-prompt
  patterns from common CLI agent tools (this needs a small, maintained
  pattern list — expect to add to it as real agents are tested against it,
  per the Phase 6 acceptance criteria referencing "a Claude Code/Codex-style
  CLI agent run directly in the terminal").
- Treat false positives/negatives in this detector as real bugs to track,
  not as inherent unsolvable fuzziness — the whole point of the product is
  that this is trustworthy enough to "close the laptop" on.

### 7.3 Socket API surface (external agents driving this)

Exposed over the same local Unix socket (or remote-equivalent) as the
control protocol, with a stable, documented subset of the control protocol
intended for external tools (not just our own client) to use:

- List workspaces / tabs / panes, with each pane's current state.
- Read a pane's recent output (some bounded amount of scrollback/recent
  text).
- Send input to a specific pane.
- Create a new workspace/tab/pane, optionally running a given command.
- Subscribe to state-change events for a pane (so an external agent doesn't
  need to poll).

## 8. Security/trust model (placeholder — finalize before Phase 8 ships)

The local socket should be filesystem-permission-restricted to the owning
user. The SSH-stdio remote path inherits whatever auth SSH itself already
enforces — do not build a separate auth layer on top assuming SSH already
gates access to the host. This is a placeholder note, not a finished
design — flag this section as needing real review before any Phase 8 work
that exposes the socket API more broadly (e.g. to other users on a shared
machine) ships.

## 9. Open decisions (do not silently resolve these — ask)

- Exact "acknowledgement" semantics for the `done` state (see § 7.1).
- Whether damage-region computation lives server-side or client-side (see
  § 6.2).
- Exact socket path convention and config file format for server discovery
  by clients (placeholder only; not yet decided).
