#!/usr/bin/env bash
# PreToolUse: blocks dangerous Edit, Write, or Bash calls.
# Exit 2 = hard-fail, blocks the tool call with the printed message.

input=$(cat)

block() {
    echo "BLOCKED: $1" >&2
    exit 2
}

tool=$(echo "$input" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_name', ''))
except Exception:
    pass
" 2>/dev/null || true)

# ── Write only (full-file overwrite risk) ───────────────────────────────────
# Edit is allowed — it makes targeted changes and is required for doc hygiene.
# Write replaces the entire file and is the actual overwrite risk being guarded.
if [ "$tool" = "Write" ]; then
    path=$(echo "$input" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('file_path', ''))
except Exception:
    pass
" 2>/dev/null || true)

    if echo "$path" | grep -qiE '(^|/)CLAUDE\.md$'; then
        block "Write (full overwrite) to CLAUDE.md is not allowed. Use Edit to make targeted changes."
    fi

    if echo "$path" | grep -qiE '(^|/)docs/'; then
        block "Write (full overwrite) to docs/ is not allowed. Use Edit to make targeted changes."
    fi
fi

# ── Bash ────────────────────────────────────────────────────────────────────
if [ "$tool" = "Bash" ]; then
    cmd=$(echo "$input" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('command', ''))
except Exception:
    pass
" 2>/dev/null || true)

    [ -z "$cmd" ] && exit 0

    # Force-push
    if echo "$cmd" | grep -qE 'git\s+push\s+(.*\s)?(--force|-f)(\s|$)'; then
        block "Force-push is not allowed. Commit your checkpoint and ask the project owner to push."
    fi

    # Deleting or overwriting CLAUDE.md via shell
    if echo "$cmd" | grep -qE '(rm|unlink)\s+.*CLAUDE\.md'; then
        block "Deleting CLAUDE.md via shell is not allowed."
    fi
    if echo "$cmd" | grep -qE '(>>?|tee)\s+CLAUDE\.md'; then
        block "Overwriting CLAUDE.md via shell redirect is not allowed."
    fi

    # Deleting or overwriting docs/ via shell
    if echo "$cmd" | grep -qE 'rm\s+(-[a-z]*\s+)*docs/'; then
        block "Deleting docs/ via shell is not allowed."
    fi
    if echo "$cmd" | grep -qE '(>>?|tee)\s+docs/'; then
        block "Overwriting docs/ via shell redirect is not allowed."
    fi

    # Deleting .git
    if echo "$cmd" | grep -qE 'rm\s+(-[a-z]*\s+)*\.git(\s|/|$)'; then
        block "Deleting .git is not allowed."
    fi
fi

exit 0
