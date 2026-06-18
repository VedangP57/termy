# Launch prompt — paste this as your first message when starting an autonomous session

```
Read CLAUDE.md and every file it imports under docs/ in full before doing
anything else. Those documents are the spec for this project — follow them
exactly, including the rejected-alternatives sections.

You're running unattended (permission prompts are disabled), so the rules
below replace me clicking "approve":

1. Work through docs/03-BUILD-PHASES.md in order, starting at Phase 1.
   Don't skip ahead to a later phase's work even if it seems faster to do
   something out of order.

2. Before marking any phase done, verify every item in that phase's
   acceptance criteria checklist actually passes — don't check a box
   because the code compiles, check it because you ran the thing the
   criterion describes.

3. Run `cargo test --workspace` and the relevant parts of the manual test
   matrix in docs/05-CONVENTIONS-AND-TESTING.md before considering a phase
   complete.

4. `git commit` after each phase's acceptance criteria are met, with a
   message naming the phase. This is your checkpoint — commit before
   starting risky or destructive operations too (e.g. before any large
   refactor or file deletion), not just at phase boundaries.

5. Do not silently resolve anything listed under an "Open decisions" heading
   in any doc (see docs/04-CLIENT-SERVER-AND-AGENT-PROTOCOL.md § 9 as an
   example). Stop and write a clear question about it in QUESTIONS.md at the
   repo root instead of guessing, then move on to other work that doesn't
   depend on that decision while it's pending.

6. If you find a real reason to deviate from a hard rule or locked decision
   in the docs, don't just do it — write the reasoning to QUESTIONS.md and
   keep working within the existing decision until it's confirmed changed.

7. Keep the docs in sync: if implementation reveals a new open decision or a
   doc goes stale, update it in the same commit as the code change, per
   docs/05-CONVENTIONS-AND-TESTING.md § 6.

Start now with Phase 1.
```

## How to start an autonomous session

```bash
cd /Users/sarvadhisolution/Documents/Personal/Termy
claude --dangerously-skip-permissions
```

Then paste the prompt above as your first message.

## Check in on progress without babysitting

- `cat QUESTIONS.md` — open decisions Claude surfaced, needs your input
- `git log --oneline` — phase checkpoints committed so far
- `cargo test --workspace` — current test state
