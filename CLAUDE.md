# CLAUDE.md — operating contract for this repository

> Last updated: 2026-06-27 · Maintained by: the working agent/human on each session.
> This file is auto-loaded into every Claude Code session. Keep it high-signal.
> If you change how this repo is built, secured, or documented, update this file in the
> same change and date the edit.
>
> **Resuming on a compute box?** Read §8 first — the web-sandbox deviations (no docker daemon,
> no gitleaks) no longer apply, and there are concrete first steps.

## 1. What this is

CEC Inventory: a standalone shop inventory system. Receipt → structured purchase records →
serialized units and bulk stock → two-layer warranty, RMA lifecycle, system re-validation,
and carrier order tracking.

- **Design source of truth:** `docs/CEC-Inventory-System-Scope.md` (the spec; everything
  hangs off its Section 4 schema and Section 20 phase plan). Do not contradict it; if reality
  diverges, record the decision in `docs/DECISIONS.md` and update the scope.
- **Build/spin-up procedure:** `AGENT_RUNBOOK.md` (ordered, with acceptance gates A–F).
- **Secret + database hygiene:** `SECRETS-AND-DATABASE.md` (read before the first commit).
- **API contract:** `docs/API.md` (full endpoint catalog) + `docs/INTEGRATION.md` (how an
  external app integrates: bearer tokens, the cec.direct seam, export, conventions).
- **Audit + remediation status:** `docs/AUDIT-2026-06-27.md` (security + data-integrity/backups
  panels; what's fixed vs the remaining lower-priority backlog).

The current state lives in **`docs/HANDOFF.md`** (what is done/blocked) and the work queue in
**`docs/TODO.md`**. Read those two before doing anything.

## 2. Hard rules (secret hygiene) — non-negotiable

These mirror `AGENT_RUNBOOK.md` §"Hard rules" and `SECRETS-AND-DATABASE.md`. Breaking one is a
stop-the-line event.

1. `.gitignore` ignores `.env`. Verify with `git check-ignore .env` (must print `.env`) before
   committing anything. It is the first file in the repo for this reason.
2. Real credentials live **only** in the gitignored `.env`, produced by
   `scripts/gen_secrets.sh`. Never write a real password/key into a committed file.
3. `.env.example` and `services/extractor/.env.example` are placeholders and ARE committed.
   `.env` is NOT. Database data lives in a Docker named volume; never commit a dump
   (`*.dump`, `*.sql.gz`) or a data dir (`pgdata/`).
4. Before any commit: confirm `.env` is not staged
   (`git status --porcelain | grep -E '(^|/)\.env$'` returns nothing) and run the secret scan
   (`gitleaks detect --source . --redact`, or `just scan`). The pre-commit hook and CI enforce
   this mechanically (§4).
5. If a secret was ever committed: rotate it first (`scripts/gen_secrets.sh` + `ALTER USER …`),
   then purge history. See `SECRETS-AND-DATABASE.md` §8.

## 3. Memory & documentation protocol (REQUIRED)

This repo keeps durable, append-style memory documents so any later agent or human can resume
without guessing. Each has a required cadence. **Touching the listed triggers without updating
the doc is incomplete work.**

| Document | Holds | You MUST update it when… |
|---|---|---|
| `CLAUDE.md` (this file) | The operating contract: rules, layout, how-to, this protocol. | Build/security/doc conventions change, or a new memory doc is added. |
| `docs/HANDOFF.md` | Point-in-time state: phase/gate status, what runs, what is blocked and why. | You finish a session, pass/fail a gate, or change the running state. Add a new dated entry; never overwrite history. |
| `docs/TODO.md` | The work queue: open items, owners, blockers. | You start, finish, block, or discover work. Tombstone completed items (§3.2). |
| `docs/DECISIONS.md` | Locked decisions and their rationale, each mapped to an `INV-OQ`. | You make or reverse a scaffold/architecture decision. Tombstone reversed ones. |
| `CHANGELOG.md` | Human-facing history of notable changes. | You ship a notable change (Keep a Changelog format). |

### 3.1 Dating (REQUIRED on every entry)

- Use ISO-8601 dates: `[YYYY-MM-DD]`. Every list entry, decision, and handoff carries one.
- Every document header carries a `Last updated: YYYY-MM-DD` line, bumped on each edit.
- When known, attach the actor: `[2026-06-27] (agent: <branch/name>)`.

### 3.2 Tombstoning (REQUIRED — do not silently delete)

Load-bearing items are never deleted; they are **tombstoned** in place so the record stays
auditable (the same append-only ethos as the `unit_event` table). Mark state with a dated tag:

- `✅ DONE [YYYY-MM-DD]` — completed. Move to the doc's "Done / Tombstoned" section.
- `🪦 TOMBSTONED [YYYY-MM-DD] — superseded by <ref>` — obsolete/replaced. Keep the original
  text, strike it through or quote it, and point to what replaced it.
- `⛔ BLOCKED [YYYY-MM-DD] — <reason>` — cannot proceed; record the blocker and the unblock
  condition.
- `◐ PARTIAL [YYYY-MM-DD] — <what is done vs outstanding>` — partially satisfied: the actionable
  part is done but a portion is outstanding or has a backstop. Record both halves.

Only purely ephemeral scratch (typos, reworded sentences) may be edited in place without a
tombstone. Anything someone might later ask "what happened to X?" gets a tombstone.

## 4. Configuration hooks present

- **Git pre-commit** (`.githooks/pre-commit`, activated via `git config core.hooksPath
  .githooks`): blocks staged `.env`/keys/dumps and runs `gitleaks protect --staged` when
  installed. Defense in depth per `SECRETS-AND-DATABASE.md` §7.
- **Claude Code SessionStart** (`.claude/settings.json` → `.claude/hooks/session-start.sh`):
  on every session it (a) verifies `.env` is ignored and not tracked, (b) re-asserts
  `core.hooksPath=.githooks`, (c) warms the cargo cache so build/lint/test are ready. Runs
  synchronously so dependencies are present before the agent loop starts.
- **CI** (`.github/workflows/ci.yml`) — five jobs: `secret-scan` (gitleaks), `rust`
  (`fmt --check` / `clippy -D warnings` / `build`, `SQLX_OFFLINE`), `tests` (full integration
  suite against a Postgres service), `compose` (builds the whole stack and smoke-tests
  `/readyz` + extractor `/health`), and `supply-chain` (`cargo audit` + `pip-audit`,
  non-blocking). `.github/dependabot.yml` opens weekly cargo/pip/docker/actions update PRs. CI
  is the real backstop: a committed secret (or a broken query, since `tests` runs every query
  against the live schema) fails the build.

## 5. How to work

```sh
just secrets    # write DB password + session secret into the gitignored .env
just up         # docker compose up -d db   (target box; needs the docker daemon)
just migrate    # sqlx migrate run           (needs sqlx-cli + a live DB)
just prepare    # cargo sqlx prepare --workspace → writes .sqlx/ (commit it)
just run        # cargo run -p cec-inventory-api  → /health, /readyz
just fmt / just lint / just scan
```

- **Database:** on a real box use `docker compose up -d db` (Postgres 16 in the `pgdata` named
  volume). Health: `docker compose ps` shows `db` healthy. The API reads `DATABASE_URL` from
  the environment and runs `migrations/` via `sqlx::migrate!` on startup.
- **`.sqlx/` offline cache:** generated by `cargo sqlx prepare --workspace` and committed. It
  only contains entries for compile-time-checked `query!`/`query_as!` macros. The Phase 0 spine
  uses none yet, so `.sqlx/` is intentionally absent until Phase 1 adds checked queries — the
  build is already DB-free. **Regenerate `.sqlx/` after any query change.**
- **Migrations are append-only:** never edit an applied migration; add a new numbered file.
  Current set: `0001_init` (18 tables) · `0002_app_user` · `0003_integrity_hardening`
  (serial/asset-tag uniqueness + append-only triggers) · `0004_app_user_role` (RBAC) ·
  `0005_api_token`. NOTE: `sqlx::migrate!` embeds at COMPILE time and won't reliably re-embed a
  new migration on stable Rust unless `crates/api/src/lib.rs` (the macro site) recompiles —
  touch it or `cargo clean -p cec-inventory-api` (a comment by the macro documents this).
- **Event logging:** write a `unit_event` row on every unit mutation as each feature lands
  (scope §16). It is the integrity backbone for RMA/transfer disputes; the audit tables are
  now **append-only at the DB level** (trigger, migration 0003) — UPDATE/DELETE on them raises.
- **Auth:** operators use argon2 + signed-cookie sessions (12 h TTL, login throttle, RBAC
  `admin`/`operator`); external apps use **bearer API tokens** (`/auth/tokens`, admin-only to
  mint). Cookie writes are CSRF-guarded (same-origin). First run: `POST /auth/bootstrap` makes
  the first admin. See `docs/INTEGRATION.md`.
- **Backups:** `just backup` (or `scripts/db_backup.sh`) dumps the DB **and** the receipt object
  store (the legal RMA proof artifacts) as a paired set; supports age encryption + retention.
  `scripts/restore_drill.sh` proves the latest backup restores. Schedule with
  `scripts/systemd/cec-backup.{service,timer}`.

## 6. Layout

```
crates/api       Rust + Axum backend (lib+bin) + server-rendered HTMX UI. Health spine; full
                 CRUD (catalog, purchases + line items + receipt upload, units + event timeline,
                 stock); landed-cost + shipments; warranty + RMA lifecycle; systems (deliver/
                 validate/sweep/transfer, row-locked); cec.direct seam (availability/reserve/
                 consume); serial verify + asset tags; identity resolution + bundles;
                 reorder/reconciliation/export; receipt-extraction seams (from text/image/
                 payload). Auth (auth.rs): argon2 sessions (TTL + login throttle + RBAC) AND
                 service-account bearer tokens; CSRF same-origin guard; per-route body limits.
crates/tracking  carrier-provider trait + poll engine (scope §12); shared by api + poller
crates/poller    shipment-tracking worker: polls active shipments via crates/tracking
crates/domain    shared domain types mapping to the Postgres enums
migrations        SQLx migrations (append-only): 0001 init (18 tables) · 0002 app_user ·
                 0003 integrity (serial/asset-tag uniqueness + append-only triggers) ·
                 0004 app_user role (RBAC) · 0005 api_token
services/extractor  Python FastAPI extractor: deterministic template path + vision.py
                 (stub | `claude` hosted-vision image backend). Local VLM/OpenCV on the GPU box.
docs              scope spec + memory docs (HANDOFF, TODO, DECISIONS) + API.md + INTEGRATION.md
                 + AUDIT-2026-06-27.md
scripts           gen_secrets / db_backup (encrypt + retention) / db_restore / restore_drill /
                 systemd/cec-backup.{service,timer}
.claude, .githooks, .github/dependabot.yml   configuration hooks (§4)
```

## 7. Build phases (scope §20)

**All phases (0–5) are implemented, tested, and hardened.** The backend covers every scope
section; there is a real operator UI (login + forms + workflow actions + scan island) and a
documented external-app integration surface (bearer tokens + the cec.direct seam). Two parallel
audit panels (security + data-integrity/backups) ran, and every Critical/High finding is
remediated across migrations 0003–0005 + auth/CSRF/backup work — see `docs/AUDIT-2026-06-27.md`.
CI runs five jobs + Dependabot (§4). Remaining follow-ups — compile-time-checked SQLx + `.sqlx/`
(D-010); the **local** VLM + OpenCV stitching on the GPU box; real carrier providers; WASM scan
fallback + guided capture; and the lower-priority audit backlog (per-IP rate limiting, session
revocation, token scopes, read-only FS, WAL/PITR + offsite, digest pinning) — are in
`docs/TODO.md`.

## 8. Running on the compute box (the deviations are gone)

This repo was built in a headless web sandbox with **no docker daemon** and **no gitleaks
binary** (deviations V-001/V-002 in `docs/DECISIONS.md`), so Postgres ran as a native local
cluster and the container path was verified only on CI. **On a real compute box those no longer
apply** — first steps:

1. `just secrets` (writes the gitignored `.env`), then `docker compose up -d --build` brings up
   the whole stack (db + extractor + api + poller). The CI `compose` job already proves this
   builds + comes up healthy, so **gate E is closeable locally** now.
2. Install `gitleaks` and run `just scan` to close gate F locally (CI runs it regardless).
3. Bootstrap the first admin (`POST /auth/bootstrap`), then mint an API token for any external
   app (`POST /auth/tokens`); see `docs/INTEGRATION.md` and `docs/API.md`.
4. **GPU/inference box:** enables the real vision path — set `EXTRACTOR_VLM_BACKEND=claude`
   (hosted interim) or wire the local model + OpenCV stitching in `services/extractor/`.
5. Stand up scheduled backups: install `scripts/systemd/cec-backup.{service,timer}` and set
   `BACKUP_AGE_RECIPIENT` (encryption) + an offsite target.
