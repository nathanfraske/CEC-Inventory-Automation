# HANDOFF — current state of the build

> Last updated: 2026-06-27 · Append a new dated entry per session; never overwrite history.
> Read this with `docs/TODO.md` (work queue) and `CLAUDE.md` (operating contract).
> Dating + tombstoning rules: `CLAUDE.md` §3.

---

## Entry [2026-06-27] — Handoff to a compute box; PR #1 merged (agent: claude/runbook-setup-config-oxwath)

End-of-session state before the project moves from the headless web sandbox to a real compute
box. **PR #1 (`claude/runbook-setup-config-oxwath` → `main`) merged with CI green** (head
`db19c2c`, run #61: all five jobs — secret-scan, rust, tests, compose, supply-chain — passed).

### Where the system is
Functionally complete and hardened. The whole scope (§20 phases 0–5) is implemented and tested;
there is a real operator UI and a documented external-app integration surface. Current test
counts: **10 unit + 17 integration** green on a fresh DB; `fmt`/`clippy -D warnings` clean.

- **Backend:** catalog, purchases (+line items, receipt upload, landed-cost), shipments+poller,
  units (+event timeline, verify, asset tags), warranty + RMA lifecycle, systems (deliver/
  validate/sweep/transfer, row-locked), cec.direct seam, identity resolution + bundles,
  reorder/reconciliation/export, receipt-extraction seams (text/image/payload).
- **Auth:** argon2 + signed-cookie sessions (12 h TTL, login throttle, RBAC admin/operator) AND
  **service-account bearer tokens** (`/auth/tokens`); CSRF same-origin guard on cookie writes;
  per-route body limits. Migrations 0001–0005.
- **UI:** dashboard + read tables + detail pages with workflow actions + entry forms + login +
  scan island + a receipt-capture block.
- **Extractor:** deterministic template path + `vision.py` (stub default; `claude` hosted-vision
  interim). Local VLM/OpenCV is the GPU-box follow-up.
- **Ops:** full `docker compose` stack (non-root, hardened); backups cover DB **and** the
  receipt object store, with encryption/retention/scheduling + a tested restore drill.
- **Docs:** `docs/API.md` (endpoint catalog), `docs/INTEGRATION.md` (external integration),
  `docs/AUDIT-2026-06-27.md` (audit + remediation status). CLAUDE.md §8 = compute-box first steps.

### Deviations that END on the compute box
- **V-001 (native Postgres instead of compose `db`)** — on a box with the docker daemon, use
  `docker compose up -d --build`. The CI `compose` job already proves the stack builds + comes
  up healthy, so **gate E is closeable locally**.
- **V-002 (manual gitleaks)** — install `gitleaks`, run `just scan` to close gate F locally.

### First steps on the new box (also CLAUDE.md §8)
`just secrets` → `docker compose up -d --build` → `POST /auth/bootstrap` (first admin) → mint an
API token for any external app. For receipt vision, set `EXTRACTOR_VLM_BACKEND=claude` or wire
the local VLM. Install the backup timer + set `BACKUP_AGE_RECIPIENT` + an offsite target.

### Open (lower priority; `docs/TODO.md` + audit doc)
Compile-time SQLx + `.sqlx/` (D-010); local VLM + OpenCV stitching; real carrier providers; WASM
scan fallback + guided capture; and the audit backlog (per-IP rate limiting, server-side session
revocation, per-endpoint token scopes, read-only container FS, WAL/PITR + automated offsite,
money f64→string in the extractor path, session-derived event actor, image/Actions digest pinning).

---

## Entry [2026-06-27] — Audit remediation, 4 batches (agent: claude/runbook-setup-config-oxwath)

Worked the audit backlog (`docs/AUDIT-2026-06-27.md`) in four verified commits after the owner
chose **globally-unique serials** and **all four batches**. Each batch: build + `fmt --check` +
`clippy -D warnings` + `cargo test --workspace` (10 unit + 17 integration) green on a **fresh**
DB, then pushed.

- **Batch 1 — integrity constraints (migration 0003).** Serial numbers globally unique (partial
  unique index; dup → 409; D-017), `stock_item.asset_tag` unique, and append-only triggers on
  `unit_event`/`system_validation`/`system_transfer`/`shipment_event`. Tests use unique-per-run
  serials (`sn()`); new `serial_number_globally_unique` test.
- **Batch 2 — DB-integrity guards.** `lock_system()` row-locks (`FOR UPDATE`) inside the tx for
  all six system mutators (fixes deliver/transfer/sweep/member TOCTOU); `units.change_status`
  enforces a transition matrix (`scrapped` terminal, etc.); `rma.update_rma` blocks reopening a
  closed case; `/export` adds the missing business tables (excludes `app_user` credentials).
- **Batch 3 — auth/access (migration 0004).** Signed-cookie session TTL (12 h); in-memory login
  throttle (10 fails → 15-min lock, 429); RBAC `admin`/`operator` with the bootstrap account as
  admin and `POST /auth/users` behind `require_admin`. Auth test extended for the role gating.
- **Batch 4 — container/supply-chain.** Non-root images (uid 10001); compose `cap_drop:[ALL]` +
  `no-new-privileges` + mem/pids limits; CI `supply-chain` job (`cargo audit` + `pip-audit`,
  non-blocking) + `.github/dependabot.yml`.

### Gotchas recorded
- `sqlx::migrate!` embeds at COMPILE time and doesn't reliably re-embed a new migration on
  stable Rust unless `lib.rs` (the macro site) is recompiled — touch it / `cargo clean -p` (a
  comment there documents this). CI rust-cache could otherwise skip embedding a new migration.
- A persistent dev DB now needs a clean slate after the uniqueness migration (dedup or
  drop/recreate). `DROP DATABASE … ; CREATE …` must be **two** `psql -c` calls (a single combined
  `-c` runs in one tx and fails for DATABASE ops).

### Remaining audit backlog (all the `◐`/`⬜` items in docs/AUDIT-2026-06-27.md)
Server-side session revocation (session table), per-IP/persistent login limiting, finer RBAC +
`/export` admin-gating + session-derived `actor`, CSRF tokens for multipart, read-only container
FS + db cap hardening, base-image/Actions digest pinning + flip audit to blocking, backup
automation/encryption/offsite + restore drill + WAL/PITR, money f64→string in the extractor
path, cross-table asset-tag uniqueness, policy-lookup unique constraints, argon2 param pinning.

---

## Entry [2026-06-27] — Security + data-integrity/backups audit panels + remediation (agent: claude/runbook-setup-config-oxwath)

Ran two parallel reviewer panels (3 security lenses: secrets/auth/session, injection/input,
infra/supply-chain; 2 data lenses: schema/transactions, backups/DR) over the whole system,
including the new vision code. Full report + prioritized backlog: **`docs/AUDIT-2026-06-27.md`**.

### Fixed this pass (one verified commit)
- **[CRITICAL] Object store now backed up.** `db_backup.sh` archives `STORAGE_FS_ROOT`
  timestamp-paired with the pg_dump (+ dump validation, `set -euo pipefail`); `db_restore.sh`
  restores the paired objects archive. Receipts (RMA legal proof) are no longer lost on restore.
- **[HIGH] `SESSION_SECRET` fails closed** — removed the committed dev-default + zero-padding
  (a forgeable known key); requires ≥64 bytes. CI `tests` job supplies a throwaway value.
- **[HIGH] Body-size limits** — global 1 MiB + 25 MiB on upload routes (also fixes the
  2 MiB-default rejecting real photos).
- **[HIGH] Concurrency/atomicity** — `direct.rs` reserve/consume locks inside the tx (was a
  released-lock double-reserve race); `rma.rs::update_rma` + `scan.rs::unit_label` now do
  mutation + event in one tx.
- **[HIGH] Operator regex** compiled with a size limit; **extraction line items validated** +
  payload/count caps; **from-image** media-type whitelist + field caps; **password floor 8→12**;
  **constant-time login** for unknown users.

### Verification
`fmt`/`clippy -D warnings` clean; `cargo test --workspace` → 10 unit + 16 integration green
(auth test password bumped to ≥12; tests source the 64-char `SESSION_SECRET` from `.env`);
python extractor + vision tests green; backup scripts `bash -n` clean.

### Owner decision blocking one HIGH fix
**Serial-number uniqueness semantics** — globally unique? per-product? refurb reuse allowed?
Needed before adding the unique index + 409 path. Raised in chat.

### Open backlog
Rate-limiting, session expiry/revocation, RBAC, append-only DB enforcement, system-gating TOCTOU
locks, status-transition matrix, `/export` completeness, asset-tag uniqueness, backup
automation/encryption/offsite + restore drill + WAL/PITR, non-root containers + resource limits,
`cargo audit`/`pip-audit`/dependabot + image-digest pinning, vision egress audit log. See
`docs/AUDIT-2026-06-27.md` and `docs/TODO.md`.

### Also fixed this session: CI compose regression
The vision commit broke the CI `compose` job (the extractor Dockerfile didn't copy the new
`vision.py`, so the container crashed on import → healthcheck timeout). Fixed in a focused commit
(`COPY extractor.py vision.py app.py`).

---

## Entry [2026-06-27] — Interim image-vision receipt path (agent: claude/runbook-setup-config-oxwath)

Closed the "need a GPU box to read receipt images" gap with an interim hosted-vision path, plus
a generic seam so an in-the-loop vision pass (operator or agent) can feed the loop right now.

### What landed
- **`services/extractor/vision.py`** — `extract_image(bytes, media_type, vendor_hint)` →
  §11.4 JSON. Backend via `EXTRACTOR_VLM_BACKEND`: `stub` (default, hermetic) or `claude`
  (POSTs the image to the Anthropic Messages API, parses + normalizes the JSON). Pure stdlib;
  the HTTP call is an injectable `_transport` so the parse/normalize path is unit-tested with no
  network. `POST /extract-image` added; `/health` reports the active backend.
- **Rust seam** (`crate::extractor`): `extract_image` client; the purchase-persist logic
  refactored into a shared `persist_extraction`; new routes `POST /purchases/from-image`
  (multipart photo → draft purchase via the vision backend) and `POST /purchases/from-payload`
  (persist a **caller-supplied** §11.4 payload — the seam for an external/operator/agent vision
  pass; no Python service needed).
- **UI**: `/ui/new` gained a *Receipt → draft purchase* block (paste text → `from-extraction`,
  or upload a photo → `from-image` via a multipart `cecUpload` helper).
- Config: `EXTRACTOR_VLM_BACKEND` / `EXTRACTOR_VLM_MODEL` / `ANTHROPIC_API_KEY` placeholders in
  both `.env.example`s; passed through to the extractor container in compose. New dep `base64`.

### Design note (D-015)
The `claude` backend is the **interim** until the local VLM is wired; it is opt-in, off by
default, key-from-env (never baked), and sends the image to a third-party API (a privacy
trade-off recorded for the security panel). `from-payload` keeps the loop extractor-agnostic —
the §11.4 JSON is the only contract.

### Verification (this environment)
- `fmt`/`clippy -D warnings` clean; `cargo test --workspace` → **10 unit + 16 integration**
  green (added `from_payload_persists_supplied_extraction`). Python: `test_vision.py` (hermetic,
  injected transport) + the existing extractor tests pass.
- Live: uvicorn extractor `/health` shows `vlm_backend: stub`; `/extract-image` returns the
  stub result; and the full **multipart** path `POST /purchases/from-image` through the auth-on
  API (login → cookie → upload) created a draft purchase (201). `docker compose config` renders
  the new extractor env.

### Still open
- The **local** VLM on the inference box (so images stay on-prem) + OpenCV long-receipt
  stitching (`/stitch` placeholder). Body-size limits for large photos through multipart/JSON
  are a follow-up. See `docs/TODO.md`.

### For live testing in-session
`POST /purchases/from-payload` is the path to use the agent's own vision as the interim: read a
receipt image, produce the §11.4 JSON, POST it → a draft purchase with unresolved lines.

---

## Entry [2026-06-27] — Operator front-end: login + entry forms + workflow actions (agent: claude/runbook-setup-config-oxwath)

Turned the read-only UI into a usable operator front-end. Previously you could look (dashboard
+ read-only tables + scan island) but not *do*; now an operator logs in and drives the system
from the browser.

### What landed (all in `crates/api/src/routes/ui.rs`, registered in `routes/mod.rs`)
- **Login / first-run** (`/ui/login`) — logs in via `POST /auth/login`, or bootstraps the first
  operator when `app_user` is empty. **Session-aware nav** on every page (operator + Logout, or
  a Login link), resolved from the signed cookie server-side (`current_user`).
- **New entry** (`/ui/new`) — forms for vendor, manufacturer, product, serialized unit, bulk
  stock, and system. Product/manufacturer/vendor dropdowns are populated server-side; enum
  fields use `<select>`s of the real PG enum values.
- **New purchase** (`/ui/purchases/new`) — header + repeatable line-item rows assembled into the
  nested `CreatePurchase` JSON.
- **Unit detail** (`/ui/units/{id}`) — change-status, assign-asset-tag, and open-RMA forms, the
  full `unit_event` timeline, and a camera-verify link.
- **System detail** (`/ui/systems/{id}`) — members list + add-member, validate, deliver (starts
  the CEC clock), parts sweep (serials textarea → array), and transfer.
- A single client helper **`cecSubmit`** serializes any form to JSON (honoring `data-type`
  number/bool/lines), POSTs/PATCHes to the form's `action` with the session cookie, and renders
  the JSON result inline (green/red), optionally redirecting/reloading.

### Design note (D-014)
The UI is a thin presentation layer: **every mutation POSTs JSON to the existing auth-protected
API routes** with the browser's signed cookie — no parallel form-encoded handlers. One contract,
auth reused for free. UI pages render publicly; their actions require a logged-in session.

### Verification (this environment)
- `fmt --check`, `clippy -D warnings` clean; `cargo test --workspace` → **10 unit + 15
  integration** green. `ui_pages_render` was extended to cover the new public pages and to render
  the unit/system **detail** pages against real rows (exercising their joins/enum casts on the
  live schema).
- Live smoke against the production app (auth ON): `/ui/login` renders; anonymous `POST /vendors`
  → 401; login sets the cookie; `/auth/me` → the operator; a **cookie-authenticated** `POST
  /vendors` (the exact path a form takes) → 201; dashboard nav shows `op1 · Logout` when logged
  in and `Login` when anonymous.

### Still open (see `docs/TODO.md`)
- WASM scan fallback (Safari/iOS) + guided long-receipt capture (need a device + the inference
  box). Line-item resolve/expand and a receipt-upload control aren't surfaced in the UI yet
  (the endpoints exist). Compile-time SQLx + `.sqlx/` remains the standing D-010 follow-up.

---

## Entry [2026-06-27] — Full one-command stack + container smoke gate (agent: claude/runbook-setup-config-oxwath)

Wired the whole system to come up from a single `docker compose` and made CI prove it on every
push — closing acceptance **gate E**, which had been ⛔ BLOCKED since the scaffold (no docker
daemon in the web sandbox).

### What landed
- **`docker-compose.yml` → full stack.** Previously only `db` + `api` (poller/minio commented).
  Now `db` + `extractor` + `api` + `poller`, all healthchecked:
  - `extractor` builds from `services/extractor/`, exposes `:8900`, healthcheck via the stdlib
    `python -c urllib.request…/health` (no curl layer needed).
  - `api` reaches Postgres at **`db`** and the extractor at **`http://extractor:8900`**; its
    `DATABASE_URL` is built from the `POSTGRES_*` parts (the `.env` URL is `localhost`, host-run
    only). Healthcheck `curl -fsS /readyz`. Waits on `db` healthy + `extractor` started.
  - `poller` builds from the new `crates/poller/Dockerfile`, same `db`-host `DATABASE_URL`,
    carrier env passed through.
- **`crates/poller/Dockerfile`** — multi-stage release build (`SQLX_OFFLINE=true`), mirrors the
  api image. The api image now installs `curl` for its healthcheck.
- **`.dockerignore`** — keeps `.env`/`*.key`/`*.pem`/dumps/`target/`/`.git/` out of the build
  context so no secret can land in an image layer (the Rust images `COPY . .`). Secret hygiene
  at the container boundary (CLAUDE.md §2).
- **CI `compose` job** — `./scripts/gen_secrets.sh` → `docker compose up -d --build --wait
  --wait-timeout 300` → curl `/readyz`, `/health`, extractor `/health` → logs-on-failure →
  `down -v`. Runs on the GitHub runner's daemon, so the container path is exercised each push.

### Verification (this environment)
- `docker compose config` renders the full stack with correct interpolation: api/poller
  `DATABASE_URL` → `…@db:5432/…`, api `EXTRACTOR_URL` → `http://extractor:8900`, secrets pulled
  from the gitignored `.env`. The extractor healthcheck one-liner parses as valid Python.
- No docker daemon here (still V-001), so `up`/build run on CI, not locally. `cargo fmt` clean
  (no Rust source changed — only Dockerfiles, compose, CI, docs).

### Gate movement
- **Gate E (containers): ⛔ BLOCKED → ✅ DONE [2026-06-27]** via the CI `compose` job. The local
  dev box still substitutes a native cluster (V-001), but the committed container path is now
  CI-verified rather than unverified.

### Next
- Build the **real operator front-end** (the UI today is read-only — dashboard + tables + scan
  island, no forms, no login page, no workflow actions). HTMX manual-entry forms + login page +
  delivery/RMA/sweep actions + WASM scan fallback. This is the next slice.

---

## Entry [2026-06-27] — Phases 2–5 + remaining Phase 1 backend build-out (agent: claude/runbook-setup-config-oxwath)

Worked the rest of the scope (§20) in 11 verified chunks, each its own commit with build +
`fmt --check` + `clippy -D warnings` + tests green. The system now covers, end to end on the
backend, every phase of the scope. Current test counts: **10 unit + 13 integration** passing
against the live DB (integration tests self-skip when `DATABASE_URL` is unset, so CI stays
DB-free).

### What landed (by scope section)
- **§5 Warranty engine + RMA readiness** — `crates/api/src/warranty.rs` (pure, tested): two
  warranty clocks, `rma_eligible` + block reasons, `cec_warranty_active`. `CecWarrantyPolicy`
  CRUD; `POST /units/{id}/recompute-warranty`; `GET /units/{id}/warranty`.
- **§8/§9 Trade-in & opening-balance intake** — `routes/intake.rs`: readiness from proof.
- **§6.1/6.2 Systems + delivery** — `routes/systems.rs`: System CRUD, membership (invalidates),
  validate, `deliver` (starts the per-unit CEC clock).
- **§6.4/6.5 Sweep + transfer** — `sweep` (reconcile members) + `transfer` (gated on a clean
  sweep, per-part mfr transferability).
- **§7 RMA lifecycle** — `routes/rma.rs`: open (modes/proof/custody), update, proof-package,
  replacement intake (predecessor retire + system re-validate).
- **§19 cec.direct seam** — `routes/direct.rs`: `/availability`, reserve, consume.
- **§13 serial verify + asset tags** — `routes/scan.rs`: verification pass + regex validation
  (warn-only); `asset-tag` (ZPL label payload).
- **§3/§15 identity resolution + bundle expansion** — `/line-items/{id}/resolve` + `/expand`.
- **§12.5/§18/§20 cross-cutting** — `routes/reports.rs`: reorder, receiving reconciliation,
  `/export` (JSON) + `/export/units.csv`.
- **§11 extractor** — `services/extractor/` FastAPI (template fast-path + VLM stub),
  pure-stdlib tested + FastAPI TestClient verified; Rust seam `POST /extract-preview`.
- **§18 path 1 UI** — `routes/ui.rs`: server-rendered dashboard + tables + a camera/scan island.

### New crates/deps
`crates/tracking` (carrier provider + poll engine). Deps added: `rust_decimal`, `regex`,
`async-trait`, `reqwest` (api), axum `multipart`, sqlx `json`/`rust_decimal`. Domain enums for
every PG enum the features touch.

### Honest gaps (follow-ups, see `docs/TODO.md`)
- Data access is still SQLx **runtime** queries (no `.sqlx/`); compile-time-checked queries +
  a Postgres-service CI job that runs the integration tests remain the deliberate follow-up
  (D-010). CI today builds DB-free and does not execute the integration tests.
- The extractor VLM path, OpenCV stitching, real carrier providers, and the WASM scan
  fallback / guided long-receipt capture are stubs/islands needing the inference box, a real
  carrier account, or a device. The schema, seams, and deterministic paths are in place.

---

## Entry [2026-06-27] — Phase 1 (part): landed-cost allocation + shipment tracking (agent: claude/runbook-setup-config-oxwath)

First two Phase 1 features (scope §20), both fully verified here.

### Landed-cost allocation (scope §14)
- `POST /purchases/{id}/allocate-costs` — spreads order-level shipping + tax − discount across
  lines weighted by line total (rounding remainder on the last line so parts sum to the whole),
  writes `allocated_landed_cost` per line, and (default) `unit_cost` per bound unit, logging a
  `note` `unit_event` per unit touched. Pure allocator in `crates/api/src/costing.rs` with unit
  tests (no DB).

### Shipment capture + polling worker (scope §12)
- New `crates/tracking` crate: a `CarrierProvider` trait, a `none` no-op and a deterministic
  `mock` provider, and the poll engine (`poll_shipment` / `poll_active_shipments`) that writes
  normalized `shipment_event` rows, advances status, sets shipped/delivered timestamps, and
  stops on delivered/returned. Idempotent (dedups by status+occurred_at).
- API: `POST /purchases/{id}/shipments`, `GET /shipments[?active=true]`, `GET /shipments/{id}`
  (with events), `POST /shipments/{id}/poll` (one tick via `CARRIER_PROVIDER`).
- `crates/poller` is now real: connects, builds the provider from env, and polls active
  shipments on the cadence (was a noop stub).
- Real carrier APIs / aggregator (USPS/UPS/FedEx/DHL/EasyPost) are the documented seam
  (`provider_from_env` falls back to `none` with a warning) — INV-OQ-30.

### Verification (this environment)
- `cargo build`/`fmt --check`/`clippy -D warnings` clean. `cargo test -p cec-inventory-api`:
  3 costing unit tests + 2 integration tests (Phase 0 + Phase 1) all pass against the live DB.
- Live curl confirmed: allocate-costs returns the 90/60 split (per-unit 1590/265, the bound
  unit updated); a single mock poll writes 4 events, sets `delivered`/`stopped` with
  shipped/delivered timestamps, and a second poll is a no-op.

### Still open for Phase 1
- Python extractor service (scope §11: stitching + template fast-path + VLM), receipt→line
  items + identity resolution + bundle expansion (§3/§15), and the camera/capture UI (§10).
  These need the inference box / a browser and are the next slices (see `docs/TODO.md`).

---

## Entry [2026-06-27] — Phase 0 CRUD: manual entry + receipt upload + event logging (agent: claude/runbook-setup-config-oxwath)

Built the "usable day one" manual-entry surface the scope's Phase 0 calls for (scope §20),
on top of the spine. The API crate is now a lib+bin (`src/lib.rs` exposes `build_state`/`app`
so the integration test drives the real router).

### What landed
- **Catalog/reference:** `POST/GET /vendors`, `GET /vendors/{id}`; `POST/GET /manufacturers`;
  `POST/GET /products`, `GET /products/{id}`.
- **Purchases (lots):** `POST /purchases` (creates the purchase + its line items in one
  transaction), `GET /purchases`, `GET /purchases/{id}` (with line items),
  `POST /purchases/{id}/line-items`, and `POST /purchases/{id}/receipt` (multipart upload to
  the filesystem object store → appends a ref to `receipt_files`).
- **Serialized units:** `POST /units` (writes an `intake` `unit_event`), `GET /units`,
  `GET /units/{id}`, `PATCH /units/{id}/status` (writes a `status_change` event with from/to),
  `GET /units/{id}/events` (the unit's event timeline).
- **Bulk stock:** `POST/GET /stock`, `POST /stock/{id}/adjust` (signed delta, guarded so
  on-hand never goes negative).
- **Event logging (scope §16):** `crates/api/src/events.rs::log_unit_event` writes a
  `unit_event` row inside the same transaction as the mutation it records.
- New domain enums in `crates/domain` (source_type, resolution_status, serial_source,
  condition_kind, acquisition_method, unit_status, unit_event_type). Money is
  `numeric(12,2)` ↔ `rust_decimal::Decimal`, sent/received as JSON strings.

### Verification (this environment)
- `cargo build --workspace`, `cargo fmt --all -- --check`, and `clippy -D warnings` all clean.
- Integration test `crates/api/tests/api.rs` (spawns the app on an ephemeral port, exercises
  the whole loop) **passes** against the live local cluster: it asserts the intake +
  status_change events, the appended receipt file, the guarded stock adjustment, and 404s.
  Direct DB check confirmed the two `unit_event` rows (`intake`→in_stock, `status_change`
  in_stock→reserved) and `receipt_files` length 1.
- The test self-skips when `DATABASE_URL` is unset, so CI (DB-free `cargo build`) stays green;
  data access still uses runtime SQLx queries, so no `.sqlx/` is required yet.

### Still open for Phase 0 / next
- No HTMX UI yet (scope §18 path 1) — API + JSON only. Minimal operator UI is the next slice.
- Compile-time-checked queries + committed `.sqlx/`, and a Postgres service in CI to actually
  run the integration test, are follow-ups (see `docs/TODO.md`, `docs/DECISIONS.md` D-010).

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
