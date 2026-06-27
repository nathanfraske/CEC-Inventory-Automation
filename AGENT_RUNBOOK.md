# CEC Inventory: Agent Runbook

Version: 1.0.0
Status: working basis.
Audience: an autonomous coding agent (or a human) standing up the repository from
the seed package in `seed/`. Ordered, with an acceptance gate at the end of each phase.
Do not skip a gate. Stop and report if a gate fails.

The design source of truth is `docs/CEC-Inventory-System-Scope.md`. This runbook builds
the Phase 0 spine from that scope (see scope Section 20 for the phase plan).

---

## 0. Scaffold decisions (locked for this build, all flippable)

These resolve the scope's open questions at the level needed to scaffold. Each maps to an
INV-OQ and can be changed later without re-architecting.

- Backend: Rust + Axum, data access via SQLx (INV-OQ-1).
- Database: PostgreSQL 16.
- Object storage: filesystem behind the API; MinIO is a drop-in later (INV-OQ-2).
- Frontend: Axum server-render + HTMX + small JS islands for the camera and scan widgets
  (INV-OQ-3). The scaffold ships the API and health spine only; UI is Phase 1+.
- Repo shape: a Cargo workspace monorepo. Crates: `api`, `poller`, `domain`. The Python
  extractor/stitcher lives under `services/extractor` and deploys to the inference box.
- License: Apache-2.0.

If any of these is wrong for you, change the seed file before Phase A and proceed.

---

## Hard rules for the agent (secret hygiene)

1. Create `.gitignore` as the FIRST file in the repo, before anything else. Verify it
   ignores `.env` before creating or copying any env file.
2. Never write a real credential into any file that will be committed. The only place a
   real password or key exists is the gitignored `.env`, produced by `scripts/gen_secrets.sh`.
3. Do not run `git add .` or `git commit` until the Phase F secret-scan gate passes.
4. `.env.example` and `services/extractor/.env.example` carry placeholders only and ARE
   committed. `.env` is NOT.
5. Database data lives in a Docker named volume, never in the working tree. Never commit a
   dump (`*.dump`, `*.sql.gz`).
6. Full detail and the production secret options are in `SECRETS-AND-DATABASE.md`. Read it
   before the first commit.

---

## Phase A: repository init

1. Choose the repo directory (default `cec-inventory`). `cd` into it.
2. Copy the seed tree into place:
   - Copy everything under `seed/` to the repo root (this includes `.gitignore`,
     `.env.example`, `docker-compose.yml`, `Cargo.toml`, `rust-toolchain.toml`, `justfile`,
     `.gitleaks.toml`, `.github/`, `crates/`, `migrations/`, `services/`, `scripts/`,
     `README.md`).
   - Copy `docs/CEC-Inventory-System-Scope.md` into `docs/` (from the project; it is the
     spec this build implements).
3. Restore executable bits on the scripts (archive transport may drop them):
   `chmod +x scripts/*.sh`
   (If a bit is still missing, the scripts have shebangs, so `bash scripts/<name>.sh` works.)
4. `git init`.
5. Confirm `.gitignore` is present at the root and contains `.env`.

Acceptance gate A: `test -f .gitignore && grep -qx '.env' .gitignore` succeeds, and
`git status` shows `.env.example` as a candidate but `.env` does not exist yet.

---

## Phase B: secrets and local Postgres

1. Generate secrets into the gitignored `.env`:
   `./scripts/gen_secrets.sh`
   This creates `.env` from `.env.example`, writes a strong `POSTGRES_PASSWORD` and
   `SESSION_SECRET`, rebuilds `DATABASE_URL`, and chmods `.env` to 600.
2. Confirm `.env` is ignored: `git check-ignore .env` must print `.env`.
3. Bring up Postgres only:
   `docker compose up -d db`
4. Wait for health: `docker compose ps` shows `db` healthy (the compose healthcheck uses
   `pg_isready`).

Acceptance gate B: `git check-ignore .env` prints `.env`, and `docker compose ps` reports
the `db` service healthy. If `.env` is NOT ignored, stop: do not proceed, fix `.gitignore`.

---

## Phase C: schema and migrations

1. Install the SQLx CLI if absent:
   `cargo install sqlx-cli --no-default-features --features rustls,postgres`
2. Run the migration against the running DB (DATABASE_URL is read from `.env`):
   `sqlx migrate run`
3. Generate the SQLx offline cache so future builds and CI need no live DB:
   `cargo sqlx prepare --workspace`
   This writes `.sqlx/`. It contains query metadata, not secrets, and IS committed.

Acceptance gate C: `sqlx migrate info` shows `0001_init` applied, and `psql "$DATABASE_URL"
-c '\dt'` (or a container `docker compose exec db psql ...`) lists the 18 tables.

---

## Phase D: build and run the API spine

1. Build the workspace:
   `cargo build --workspace`
   If a dependency major has moved on crates.io (for example axum), update the version in
   the root `Cargo.toml`, run `cargo update`, and adapt `crates/api/src/main.rs` to the new
   API. `axum::serve` exists from 0.7 on.
2. Run the API locally:
   `cargo run -p cec-inventory-api`
3. Verify the spine:
   - `curl -s localhost:8080/health` returns `ok`.
   - `curl -s localhost:8080/readyz` returns `{"db":"up"}`.

Acceptance gate D: both endpoints respond as above. `/readyz` proves the API reached
Postgres using the gitignored DATABASE_URL.

---

## Phase E: container build (optional but recommended)

1. Build and run the full stack in containers:
   `docker compose up -d --build`
   The api image builds with `SQLX_OFFLINE=true`, so it does not need a live DB at build.
2. Verify `curl -s localhost:8080/readyz` returns `{"db":"up"}` against the containerized DB.

Acceptance gate E: the api container is healthy and `/readyz` reports the DB up.

---

## Phase F: secret scan, then first commit

1. Scan the working tree for secrets before staging anything:
   `gitleaks detect --source . --redact`
   (Install gitleaks if absent. The `.gitleaks.toml` allowlists the example env files and
   the `CHANGE_ME` placeholders only.)
2. Confirm `.env` is not staged: `git status --porcelain | grep -E '(^|/)\.env$'` must
   return nothing.
3. Stage and commit:
   `git add .`
   `git status` (eyeball: no `.env`, no `*.dump`, no `pgdata/`)
   `git commit -m "chore: scaffold CEC Inventory (Phase 0 spine, schema, secrets hygiene)"`

Acceptance gate F: gitleaks reports no leaks, `.env` is absent from `git status`, and the
commit succeeds. CI (`.github/workflows/ci.yml`) re-runs gitleaks plus fmt/clippy/build on
push; if a secret was ever staged, CI fails.

---

## After Phase 0

The repo now stands up: Postgres with the full schema, a Rust API that connects and
answers health, the polling worker stub, the extractor service skeleton, and CI with secret
scanning. Build out by scope phase (scope Section 20):

- Phase 1: receipt capture + stitching + extraction + landed cost + shipment polling. Wire
  the extractor service (`services/extractor`), the HTMX capture UI, and fill the poller.
- Phase 2: barcode/serial scan, asset-tag printing, opening-balance intake.
- Phase 3: ownership, delivery, the two warranties, RMA lifecycle.
- Phase 4: system re-validation and warranty transfer (the parts sweep).
- Phase 5: the cec.direct seam.

Regenerate the SQLx cache (`just prepare`) after any query change, and add a new numbered
migration per schema change (never edit an applied migration).
