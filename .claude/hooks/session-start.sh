#!/usr/bin/env bash
# CEC Inventory — Claude Code SessionStart hook.
# Runs at the start of every session (web and local). Three jobs, in priority order:
#   1. Enforce the repo's first rule: .gitignore must ignore .env, and .env must not be tracked.
#   2. Activate the committed git pre-commit secret-scan hook (.githooks/pre-commit).
#   3. Warm the Rust dependency cache so build/lint/test run without first-run latency.
# Synchronous (deps ready before the agent loop starts), idempotent, non-interactive.
set -uo pipefail

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}" || exit 0

# --- 1. secret hygiene (hard rule #1) ---------------------------------------
if ! grep -qx '.env' .gitignore 2>/dev/null || ! git check-ignore -q .env 2>/dev/null; then
  echo "SECRET-HYGIENE FAILURE: .gitignore no longer ignores .env. Fix before any commit." >&2
fi
if git ls-files --error-unmatch .env >/dev/null 2>&1; then
  echo "SECRET-HYGIENE FAILURE: .env is TRACKED by git. Remove it, rotate the secret," >&2
  echo "  and purge history. See SECRETS-AND-DATABASE.md Section 8." >&2
fi

# --- 2. activate the committed pre-commit hook ------------------------------
if [ "$(git config --get core.hooksPath 2>/dev/null || true)" != ".githooks" ]; then
  git config core.hooksPath .githooks 2>/dev/null || true
fi

# --- 3. warm Rust deps (cached after the first run) -------------------------
if command -v cargo >/dev/null 2>&1; then
  cargo fetch --quiet 2>/dev/null || true
fi

# --- status banner (becomes session context) --------------------------------
echo "CEC Inventory ready. Orientation order: CLAUDE.md -> docs/HANDOFF.md -> docs/TODO.md."
echo "Design source of truth: docs/CEC-Inventory-System-Scope.md. Build steps: AGENT_RUNBOOK.md."
echo "Never commit .env or DB dumps. Secrets live only in the gitignored .env (gen via scripts/gen_secrets.sh)."
exit 0
