# Changelog

All notable changes to this repository are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/); dates are ISO-8601. See `CLAUDE.md` §3 for the
dating + tombstoning conventions that govern the memory documents.

## [Unreleased]

### Added — 2026-06-27 — Phase 0 scaffold
- Cargo workspace spine: `crates/api` (Axum, `/health` + `/readyz`), `crates/poller`
  (shipment-worker stub), `crates/domain` (enum types).
- `migrations/0001_init.sql`: the full Phase 0 schema (18 tables) per scope §4.
- Single-box stack: `docker-compose.yml` (Postgres 16 named volume), `crates/api/Dockerfile`
  (`SQLX_OFFLINE=true`), `justfile`, `rust-toolchain.toml`.
- Secret hygiene: `.gitignore` (env/keys/dumps/data dirs), `.env.example` +
  `services/extractor/.env.example` placeholders, `scripts/gen_secrets.sh`, `.gitleaks.toml`,
  `scripts/db_backup.sh` / `scripts/db_restore.sh`.
- CI: `.github/workflows/ci.yml` — gitleaks + `fmt`/`clippy`/`build`.
- Configuration hooks: git pre-commit secret-scan (`.githooks/pre-commit`) and a Claude Code
  SessionStart hook (`.claude/settings.json` + `.claude/hooks/session-start.sh`).
- Memory documents: `CLAUDE.md`, `docs/HANDOFF.md`, `docs/TODO.md`, `docs/DECISIONS.md`, and
  this changelog, with an explicit dating/tombstoning protocol.
- Docs: `docs/CEC-Inventory-System-Scope.md` (design source of truth), `AGENT_RUNBOOK.md`,
  `SECRETS-AND-DATABASE.md`, `README.md`.

### Notes
- Acceptance gates A–D verified in this environment; gate E (containers) and a local gitleaks
  run for gate F are infra-blocked in the web sandbox — see `docs/HANDOFF.md` entry
  [2026-06-27].
