# HANDOFF — current state of the build

> Last updated: 2026-06-27 · Append a new dated entry per session; never overwrite history.
> Read this with `docs/TODO.md` (work queue) and `CLAUDE.md` (operating contract).
> Dating + tombstoning rules: `CLAUDE.md` §3.

---

## Entry [2026-06-27] — Phase 0 scaffold stood up from the seed (agent: claude/runbook-setup-config-oxwath)

Executed `AGENT_RUNBOOK.md` end to end against the seed package, plus set up the
configuration hooks and these memory documents.

### What is in place
- Full seed tree laid down at the repo root (`.gitignore` first, per hard rule #1):
  Cargo workspace (`crates/api`, `crates/poller`, `crates/domain`), `migrations/0001_init.sql`
  (the full Phase 0 schema, 18 tables), `docker-compose.yml`, `justfile`, `rust-toolchain.toml`,
  `.gitleaks.toml`, `.github/workflows/ci.yml`, `services/extractor/`, `scripts/`, `README.md`,
  `docs/CEC-Inventory-System-Scope.md`, and the operational docs `AGENT_RUNBOOK.md` +
  `SECRETS-AND-DATABASE.md`.
- Configuration hooks: git pre-commit secret-scan (`.githooks/`) and a Claude Code SessionStart
  hook (`.claude/`). See `CLAUDE.md` §4.
- Memory docs: this file, `docs/TODO.md`, `docs/DECISIONS.md`, `CHANGELOG.md`, `CLAUDE.md`.

### Acceptance-gate results (runbook Phases A–F)
| Gate | Result | Evidence / note |
|---|---|---|
| **A** — repo init | ✅ DONE [2026-06-27] | `.gitignore` present and ignores `.env` (`git check-ignore .env` → `.env`); `.env` absent; `.env.example` is a tracked candidate; scripts `chmod +x`. |
| **B** — secrets + Postgres | ✅ DONE [2026-06-27] (DB via local cluster, see note) | `scripts/gen_secrets.sh` wrote real `POSTGRES_PASSWORD`/`SESSION_SECRET` into the gitignored `.env` (chmod 600, no `CHANGE_ME` left, absent from `git status`). DB brought up healthy. **Substitution:** the docker daemon is not reachable in this sandbox, so Postgres 16 was started as a **native local cluster** (same user `cec`, db `cec_inventory`, port 5432) instead of `docker compose up -d db`. On the target box, use compose — it is the documented path and is byte-for-byte present. |
| **C** — schema + migrations | ✅ DONE [2026-06-27] | Migration applied via the API's startup `sqlx::migrate!` (equivalent to `sqlx migrate run`): `_sqlx_migrations` shows `0001 init` success=t; `psql \dt` lists exactly **18 tables**. See note on `.sqlx/` below. |
| **D** — build + run API spine | ✅ DONE [2026-06-27] | `cargo build --workspace` clean with the pinned versions (axum 0.8.9, sqlx 0.8.6 — no adaptation needed). `curl localhost:8080/health` → `ok`; `curl localhost:8080/readyz` → `{"db":"up"}` (proves the API reached Postgres via the gitignored `DATABASE_URL`). |
| **E** — container build | ⛔ BLOCKED [2026-06-27] — no docker daemon in this sandbox | `crates/api/Dockerfile` (builds with `SQLX_OFFLINE=true`) and `docker-compose.yml` are present and correct. Run `docker compose up -d --build` and re-verify `/readyz` on a host with Docker. |
| **F** — secret scan + first commit | ◐ PARTIAL [2026-06-27] — gitleaks unavailable locally; manual scan + CI cover it | gitleaks could not be installed here (external-binary download is denied in this environment). Performed a manual staged-content secret scan and confirmed `.env`/dumps/keys are not staged before committing. The CI `gitleaks` job runs on push (the real backstop), and the pre-commit hook runs gitleaks on any machine that has it. **Run `gitleaks detect --source . --redact` locally where gitleaks is installed to fully close gate F.** |

### Important note on `.sqlx/` (offline cache)
The runbook's Phase C step 3 (`cargo sqlx prepare --workspace` → `.sqlx/`) only produces entries
for compile-time-checked `query!`/`query_as!` macros. The Phase 0 spine uses **none** (it uses
runtime `sqlx::query_scalar` and `sqlx::migrate!`), so `cargo sqlx prepare` would write an empty
cache and `.sqlx/` is **intentionally absent**. The workspace already builds with no `DATABASE_URL`
and no live DB. Generate and commit `.sqlx/` once Phase 1 introduces checked queries.

### Running state at end of session
- A local Postgres 16 cluster was running at `localhost:5432` (data under
  `/var/lib/postgresql/cec`, outside the repo) and the API binary was serving on `:8080` for
  gate verification. **These are sandbox-ephemeral** — the container is reclaimed after the
  session; nothing of it is committed. Re-create the DB on the target box with `docker compose
  up -d db` (or `just up`).

### Post-build verification [2026-06-27]
Ran an adversarial multi-reviewer pass (runbook fidelity, secret hygiene + hooks, memory-doc
quality, scope/schema consistency). Verdict: fix-then-ship, **no blockers, no secret exposure,
both hooks function**. Applied the review's polish items in the same session: hardened the
pre-commit regex to also block bare `*.env` (e.g. `prod.env`) to match `.gitignore`; defined the
`◐ PARTIAL` tag in `CLAUDE.md` §3.2; aligned gate-F status across HANDOFF/TODO/CHANGELOG; added
the `Last updated` header to `CHANGELOG.md`; and corrected the `D-001` SQLx note (Phase 0 uses
runtime queries, so `.sqlx/` is intentionally absent until Phase 1).

### CI fix [2026-06-27]
First PR CI run failed on the `secret-scan` job: `gitleaks-action@v2` now requires
`GITHUB_TOKEN` to scan `pull_request` events (the log confirmed it was a config error, not a
detected secret — "No license key is required"). Fixed by passing
`env: GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}` to the gitleaks step in
`.github/workflows/ci.yml`. The `rust` job (fmt/clippy/build) was unaffected.

### Next session should
1. On a Docker host: `docker compose up -d --build` → close gate E; run gitleaks → close gate F.
2. Begin Phase 1 (scope §20): wire object storage, the Python extractor, receipt capture +
   stitching, identity resolution, landed-cost allocation, and the shipment poller. See
   `docs/TODO.md`.
