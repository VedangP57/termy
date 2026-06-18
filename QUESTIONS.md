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
