# DECISIONS — locked choices and rationale

> Last updated: 2026-06-27 · Dating + tombstoning rules: `CLAUDE.md` §3.
> Each decision is dated and, where it resolves an open question from the scope, mapped to its
> `INV-OQ`. Decisions are "locked for this build, all flippable" (`AGENT_RUNBOOK.md` §0). When
> a decision is reversed, **tombstone** it (do not delete) and add the replacement.

## Active decisions

| # | Date | Decision | Resolves | Rationale |
|---|---|---|---|---|
| D-001 | 2026-06-27 | Backend = Rust + Axum; data access via SQLx (compile-time-checked queries). | INV-OQ-1 | Leaner than SeaORM; fits the verification posture. |
| D-002 | 2026-06-27 | Database = PostgreSQL 16. | — | Relational fits inventory; JSONB for raw extract / event detail / carrier payloads. |
| D-003 | 2026-06-27 | Object storage = filesystem behind the API; MinIO is a drop-in later. | INV-OQ-2 | Simplest for one box; `STORAGE_BACKEND` flips it. |
| D-004 | 2026-06-27 | Frontend = Axum server-render + HTMX + small JS islands (camera/scan). Scaffold ships API + health spine only; UI is Phase 1+. | INV-OQ-3 | Least total code; camera/scan are the only places the choice bites. |
| D-005 | 2026-06-27 | Repo shape = Cargo workspace monorepo, crates `api` / `poller` / `domain`; Python extractor under `services/extractor`. | — | Matches the Rust-core / Python-isolated-to-ML split. |
| D-006 | 2026-06-27 | License = Apache-2.0. | — | Stated in the runbook; the existing repo `LICENSE` already is Apache-2.0. |
| D-007 | 2026-06-27 | Secret hygiene = `.gitignore`-first, real secrets only in the gitignored `.env` (from `scripts/gen_secrets.sh`); gitleaks in CI + a git pre-commit hook; DB data in a Docker named volume, dumps outside the repo. | — | `SECRETS-AND-DATABASE.md`. The repo is public-shaped; assume anything committed leaks. |
| D-008 | 2026-06-27 | Configuration hooks = git pre-commit secret-scan (`.githooks/pre-commit`, `core.hooksPath=.githooks`) + Claude Code SessionStart hook (`.claude/`). | — | The pre-commit hook is the optional gitleaks hook called for in `SECRETS-AND-DATABASE.md` §7; the SessionStart hook enforces hygiene and warms the build cache each session. |
| D-009 | 2026-06-27 | Memory-document model = `CLAUDE.md` + `docs/HANDOFF.md` + `docs/TODO.md` + `docs/DECISIONS.md` + `CHANGELOG.md`, governed by an explicit dating + tombstoning protocol (`CLAUDE.md` §3). | — | Durable, auditable state so any later agent/human resumes without guessing; mirrors the append-only `unit_event` ethos. |

## Deviations recorded this build (sandbox-only, do not propagate to the target box)

| # | Date | Deviation | Why | Correct path on a real box |
|---|---|---|---|---|
| V-001 | 2026-06-27 | Postgres started as a **native local cluster** instead of `docker compose up -d db`. | The docker daemon is not reachable in the web sandbox. Same user/db/port/version. | `docker compose up -d db` (the documented, committed path). |
| V-002 | 2026-06-27 | Gate F secret scan done **manually**; gitleaks not run locally. | Installing the gitleaks binary is denied in this environment. | `gitleaks detect --source . --redact` locally; CI runs it on push regardless. |

> The deviations above are environment workarounds, not design decisions. The committed repo
> still describes and uses the docker + gitleaks paths exactly as written.

## Tombstoned / reversed decisions

_None yet._
