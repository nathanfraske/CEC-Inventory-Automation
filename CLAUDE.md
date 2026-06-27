# CLAUDE.md — operating contract for this repository

> Last updated: 2026-06-27 · Maintained by: the working agent/human on each session.
> This file is auto-loaded into every Claude Code session. Keep it high-signal.
> If you change how this repo is built, secured, or documented, update this file in the
> same change and date the edit.

## 1. What this is

CEC Inventory: a standalone shop inventory system. Receipt → structured purchase records →
serialized units and bulk stock → two-layer warranty, RMA lifecycle, system re-validation,
and carrier order tracking.

- **Design source of truth:** `docs/CEC-Inventory-System-Scope.md` (the spec; everything
  hangs off its Section 4 schema and Section 20 phase plan). Do not contradict it; if reality
  diverges, record the decision in `docs/DECISIONS.md` and update the scope.
- **Build/spin-up procedure:** `AGENT_RUNBOOK.md` (ordered, with acceptance gates A–F).
- **Secret + database hygiene:** `SECRETS-AND-DATABASE.md` (read before the first commit).

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
- **CI** (`.github/workflows/ci.yml`): `gitleaks` on every push/PR, plus `cargo fmt --check`,
  `clippy -D warnings`, and `cargo build` with `SQLX_OFFLINE=true`. This is the real backstop:
  a committed secret fails the build.

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
- **Event logging:** write a `unit_event` row on every unit mutation as each feature lands
  (scope §16). It is the integrity backbone for RMA/transfer disputes.

## 6. Layout

```
crates/api       Rust + Axum backend (lib+bin). Spine: /health, /readyz. Phase 0 CRUD:
                 vendors/manufacturers/products, purchases (+line items, receipt upload),
                 units (+status change, event timeline), stock. UI (HTMX) is still to come.
crates/poller    shipment-tracking worker (stub; Phase 1 fills carrier polling)
crates/domain    shared domain types mapping to the Postgres enums
migrations        SQLx SQL migrations (0001_init = the full Phase 0 schema, 18 tables)
services/extractor  Python receipt stitch + extraction service (runs on the inference box)
docs              the scope spec + memory docs (HANDOFF, TODO, DECISIONS)
scripts           gen_secrets / db_backup / db_restore
.claude, .githooks  configuration hooks (§4)
```

## 7. Build phases (scope §20)

Phase 0 (this scaffold): schema + API spine + secret hygiene + CI. Done — see `docs/HANDOFF.md`.
Phases 1–5: receipt capture/extraction/cost/tracking → scan/labels/migration → ownership,
delivery, warranties, RMA → systems/re-validation/transfer → cec.direct seam. Build by phase;
each is shippable. Keep the memory docs current as you go.
