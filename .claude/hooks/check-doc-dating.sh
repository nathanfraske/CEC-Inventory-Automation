#!/usr/bin/env bash
# CEC Inventory — Claude Code PostToolUse hook (Edit|Write|MultiEdit): enforce §3.1 dating.
#
# When a memory doc (HANDOFF / TODO / DECISIONS / CHANGELOG / CLAUDE) is edited, verify its
# `Last updated:` header is today's date. If it is stale, nudge the agent (exit 2 → stderr is
# fed back to Claude) to bump the header and follow the dating/tombstoning rules. The edit has
# already been written; this is a self-correct prompt, not a block.
set -uo pipefail

input="$(cat 2>/dev/null || true)"

fp=""
if command -v jq >/dev/null 2>&1; then
  fp="$(printf '%s' "$input" | jq -r '.tool_input.file_path // empty' 2>/dev/null || true)"
else
  fp="$(printf '%s' "$input" | grep -oE '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed -E 's/.*:[[:space:]]*"//; s/"$//')"
fi
[ -n "$fp" ] || exit 0

case "$(basename "$fp")" in
  HANDOFF.md|TODO.md|DECISIONS.md|CHANGELOG.md|CLAUDE.md) base="$(basename "$fp")" ;;
  *) exit 0 ;;
esac
[ -f "$fp" ] || exit 0

today="$(date +%F)"
hdr_date="$(grep -m1 -oE 'Last updated:[[:space:]]*[0-9]{4}-[0-9]{2}-[0-9]{2}' "$fp" 2>/dev/null \
            | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}' | head -1)"

[ -z "$hdr_date" ] && exit 0           # no parseable header date → stay silent (no false alarm)
[ "$hdr_date" = "$today" ] && exit 0   # already today → compliant

echo "Doc-dating (CLAUDE.md §3.1): ${base} still reads 'Last updated: ${hdr_date}' but today is ${today}." >&2
echo "Bump its 'Last updated:' header to ${today}; date new entries [${today}]; tombstone (don't silently delete) completed items per §3.2." >&2
exit 2
