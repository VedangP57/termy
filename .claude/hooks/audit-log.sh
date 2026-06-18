#!/usr/bin/env bash
# PostToolUse: appends every Bash command to logs/audit.log with a timestamp.
# For human review only — not read back by Claude.

input=$(cat)
cmd=$(echo "$input" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('command', ''))
except Exception:
    pass
" 2>/dev/null || true)

if [ -z "$cmd" ]; then
    exit 0
fi

mkdir -p logs
printf '[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$cmd" >> logs/audit.log
exit 0
