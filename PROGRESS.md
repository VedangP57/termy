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
