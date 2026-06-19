# Open decisions and blockers

Write here instead of guessing. Format: question, what was tried or assumed
for now, and what needs confirming before it's changed.

---

## Q1 — Socket path convention
**Source:** `docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 9 (explicitly open)

The docs say "Exact socket path convention and config file format for server
discovery by clients — placeholder only; not yet decided."

**Phase 1 placeholder used:** `$XDG_RUNTIME_DIR/termd.sock` with fallback to
`/tmp/termd-$USER.sock` when `XDG_RUNTIME_DIR` is not set (common on macOS).

**Question:** Is this the right convention, or do you want a specific path
(e.g. `~/.local/state/termd/termd.sock`, or a path with a server ID to allow
multiple instances)? Also: should the client discover the server automatically,
or always require an explicit `--socket <path>` flag?

---

## Q2 — Binary crate structure vs. single-binary hard rule
**Source:** `docs/02-TECH-STACK.md` workspace layout + `CLAUDE.md` hard rules

The workspace layout in the docs shows `crates/server-bin/` and
`crates/client-bin/` both described as "binary entry points." The hard rule
in CLAUDE.md says "single binary, two modes — never split into two separate
crates/binaries."

These conflict. **Phase 1 resolution used:**

- `crates/client-bin/` — binary crate, Cargo package name `termd`, produces
  the single `termd` binary. Handles both `--server` and default (passthrough)
  modes.
- `crates/server-bin/` — *library* crate containing server-side logic (PTY
  management, socket accept, byte relay). Depended on by `termd`.

`cargo tree -p server-bin` works on a library crate and will show no GPU
crates (there are none in Phase 1, and the rule remains enforced in later
phases by keeping render/fonts out of server-bin's deps).

**Question:** Is this the right long-term interpretation? Specifically: when
Phase 5 adds GPU code to `client-bin`, the `termd` binary will link wgpu/winit.
At that point, a server-only build (to run headless on a remote host) would
still link GPU code. Do you want a separate server binary eventually, or is
a single binary that links everything (but only exercises GPU code in client
mode) acceptable?

---

## Q3 — Done-state acknowledgement semantics
**Source:** `docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 7.1 / § 9

The docs say the `done` state means "a foreground process exited, not yet
acknowledged" but leave the acknowledgement mechanism explicitly open.

**Phase 8 implementation assumption:** `done` is automatically cleared the
next time a shell prompt is detected (i.e. shell re-enters idle). No explicit
user/client acknowledgement is required. This means `done` is a transient
state visible only in the brief window between process exit and the shell
reprinting its prompt.

**Question:** Is auto-clear on next-idle correct, or does `done` need an
explicit acknowledgement action (e.g. viewing the pane, sending a specific
API call)? If the intended UX is a persistent "this job finished" badge that
the user must dismiss, the auto-clear approach is wrong.

---

## Q4 — Damage-region computation: server-side or client-side?
**Source:** `docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md` § 6.2 / § 9

The docs defer this decision to Phase 5/8 implementation time.

**Phase 8 implementation decision:** damage computation is client-side.
Rationale: the server sends raw PTY bytes (thin framing: pane_id + length +
raw bytes). The client parses them into its local terminal model and uses its
existing `DamageTracker` to decide what to redraw. Server-side damage would
require the server to hold a second terminal model just for diffing, doubling
memory cost for no benefit — the client already owns the renderer and is the
only consumer of damage data.

**Question:** Confirm this is acceptable. If you later want the server to do
damage computation (e.g. to support multiple simultaneous clients sharing a
pane view), this decision needs revisiting.
