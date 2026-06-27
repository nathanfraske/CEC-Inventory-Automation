#!/usr/bin/env bash
# CEC Inventory — Claude Code Stop hook: enforce the §3 memory/documentation protocol.
#
# If a session changed source/ops files but left the memory docs untouched, block the
# stop ONCE and tell the agent to update docs/HANDOFF.md + docs/TODO.md (and CHANGELOG.md /
# DECISIONS.md / CLAUDE.md as warranted) per CLAUDE.md §3 — dated [YYYY-MM-DD], tombstoned.
#
# Loop-safe (honors stop_hook_active), read-only (only inspects `git status`), non-interactive.
set -uo pipefail

input="$(cat 2>/dev/null || true)"

# --- loop guard: if we already blocked this stop, let it through --------------
active="false"
if command -v jq >/dev/null 2>&1; then
  active="$(printf '%s' "$input" | jq -r '.stop_hook_active // false' 2>/dev/null || echo false)"
else
  printf '%s' "$input" | grep -qE '"stop_hook_active"[[:space:]]*:[[:space:]]*true' && active="true"
fi
[ "$active" = "true" ] && exit 0

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}" || exit 0
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

porcelain="$(git status --porcelain 2>/dev/null || true)"
[ -z "$porcelain" ] && exit 0

# strip the 2-char status + space; for renames keep the destination path
paths="$(printf '%s\n' "$porcelain" | sed -E 's/^...//; s/^.*-> //')"

SRC_RE='^(crates/|migrations/|services/|scripts/|\.github/|docker-compose\.yml|Cargo\.(toml|lock)|justfile|rust-toolchain\.toml)'
DOC_RE='(^|/)(HANDOFF|TODO|DECISIONS|CHANGELOG|CLAUDE)\.md$'

changed_src="$(printf '%s\n' "$paths" | grep -E "$SRC_RE" || true)"
[ -z "$changed_src" ] && exit 0                         # no source/ops changes → nothing to enforce
printf '%s\n' "$paths" | grep -qE "$DOC_RE" && exit 0   # a memory doc was already touched → compliant

today="$(date +%F)"
reason="Doc-compliance check (CLAUDE.md §3): this session changed source/ops files but updated no memory doc:
$(printf '%s\n' "$changed_src" | sed 's/^/  - /')

Before finishing, satisfy the §3 protocol:
  - docs/HANDOFF.md  — append a new dated [${today}] entry describing the state change (never overwrite history).
  - docs/TODO.md     — record started/finished/blocked work; tombstone completed items (✅ DONE [${today}]).
  - CHANGELOG.md     — add an entry if this is a notable change (Keep a Changelog format).
  - docs/DECISIONS.md / CLAUDE.md — if an architecture decision or a build/security/doc convention changed.
Use ISO-8601 [YYYY-MM-DD] dates (§3.1) and tombstone rather than delete (§3.2).
If no doc update is warranted (pure scratch/experiment, or changes you will discard), say so explicitly, then stop again."

if command -v jq >/dev/null 2>&1; then
  jq -n --arg r "$reason" '{decision:"block", reason:$r}'
else
  esc="$(printf '%s' "$reason" | sed 's/\\/\\\\/g; s/"/\\"/g' | sed ':a;N;$!ba;s/\n/\\n/g')"
  printf '{"decision":"block","reason":"%s"}\n' "$esc"
fi
exit 0
