# Changelog

> Last updated: 2026-06-27

All notable changes to this repository are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/); dates are ISO-8601. See `CLAUDE.md` §3 for the
dating + tombstoning conventions that govern the memory documents.

## [Unreleased]

### Security — 2026-06-27 — Audit remediation batch 3: auth/access hardening
- **Session TTL:** the signed cookie now carries its issue time; sessions expire after 12 h
  (absolute), enforced in `require_auth`/`me`. Old timestamp-less cookies read as expired.
- **Login throttle:** an in-memory per-username counter locks an account after 10 consecutive
  failures for 15 min (429), cleared on success — a brute-force speed-bump for the mesh deploy.
- **RBAC:** new `app_user.role` (migration 0004); the bootstrap account is `admin`, others
  default to `operator`. `POST /auth/users` is gated behind a `require_admin` middleware
  (operators get 403). `/auth/me` now returns the role. New 403/429 `ApiError` variants.
- Auth integration test extended to assert admin-creates-operator and operator-cannot-create.

### Security — 2026-06-27 — Audit remediation batch 2: DB-integrity guards
- **System-gating TOCTOU fixed:** a `lock_system` helper row-locks the `system` (`FOR UPDATE`)
  as the first statement inside the tx for add/remove member, validate, deliver, sweep, transfer;
  gate checks re-read the locked state and member/snapshot reads moved inside the tx.
- **Status-transition matrix:** `units.change_status` rejects illegal jumps (`scrapped` is
  terminal, `returned` must be restocked/inspected first, …); `rma.update_rma` refuses to move a
  terminal `closed` case.
- **`/export` completeness:** added `vendor_return_policy`, `cec_warranty_policy`, `trade_in`,
  `trade_in_unit`; `app_user` deliberately excluded (credentials).

### Security — 2026-06-27 — Audit remediation batch 1: integrity constraints (migration 0003)
- **Serial numbers are globally unique** (D-017): partial unique index on
  `inventory_unit.serial_number WHERE serial_number IS NOT NULL`; duplicate bind/intake → 409.
- **Bulk-stock asset tags are unique** (`stock_item.asset_tag`); units/systems already were.
- **Append-only enforced at the DB:** a `BEFORE UPDATE OR DELETE` trigger on `unit_event`,
  `system_validation`, `system_transfer`, `shipment_event` raises on any history mutation.
- Integration tests use unique-per-run serials; a `serial_number_globally_unique` test asserts
  the 409 (and that NULL serials remain exempt).

### Security — 2026-06-27 — Audit-panel remediation (see docs/AUDIT-2026-06-27.md)
Two parallel reviewer panels (security + data-integrity/backups) ran over the whole system.
Fixed this pass:
- **Backups (Critical):** `db_backup.sh` now archives the receipt/photo object store alongside
  the Postgres dump (timestamp-paired), validates the dump, and uses `set -euo pipefail`;
  `db_restore.sh` restores the paired objects archive. Previously only Postgres was dumped, so a
  restore left dangling receipt references (the legal RMA proof artifacts).
- **Auth/secret (High):** `SESSION_SECRET` now fails closed (no committed dev-default, no
  zero-padding; requires ≥64 bytes) — the old fallback was a known, forgeable signing key.
- **DoS (High):** global 1 MiB request body cap + 25 MiB on the upload routes (also fixes the
  2 MiB-default rejecting real receipt photos).
- **Concurrency/atomicity (High):** `direct.rs` reserve/consume now locks inside the tx (was a
  no-op lock → double-reserve race); `update_rma` and `unit_label` wrap mutation + event log in
  one transaction (were two autocommits).
- **Input hardening (High/Med/Low):** operator `serial_format_regex` compiled with a size limit;
  `persist_extraction` validates/caps line items + payload size; `from-image` whitelists media
  type and caps text fields; password floor 8→12; constant-time login for unknown users.

Open backlog (rate-limiting, session expiry/revocation, RBAC, serial uniqueness, append-only DB
enforcement, backup automation/encryption, non-root containers, dep-advisory scanning, …) is
tracked in `docs/AUDIT-2026-06-27.md` and `docs/TODO.md`.

### Added — 2026-06-27 — Interim image-vision receipt path (scope §11.2)
- Receipt **images** can now be extracted without the local GPU VLM. New `vision.py` backend
  selected by `EXTRACTOR_VLM_BACKEND`: `stub` (default, hermetic) or `claude` — POSTs the image
  to the Anthropic Messages API (a Claude vision model) and parses the §11.4 JSON. Key/model are
  read from the gitignored `.env`; opt-in and off by default (the image is third-party egress).
  New extractor endpoint `POST /extract-image`; `/health` now reports the active backend.
- Rust seam: `extractor::extract_image` (base64 → extractor), a shared `persist_extraction`
  helper, and two new routes — `POST /purchases/from-image` (multipart photo → draft purchase)
  and `POST /purchases/from-payload` (persist a caller-supplied §11.4 payload — the seam an
  external/operator/agent vision pass uses to feed the loop now, no Python service required).
- UI: `/ui/new` gains a **Receipt → draft purchase** block (paste text, or upload a photo).
- Tests: hermetic `test_vision.py` (injected transport) + a `from_payload_persists_supplied_extraction`
  integration test. Live-smoked the full multipart image path through the auth-on API against a
  uvicorn extractor.

### Added — 2026-06-27 — Operator front-end: login, entry forms, workflow actions
- The UI is no longer read-only. New server-rendered pages (still no template-engine dep):
  - **Login / first-run** (`/ui/login`): logs in via `POST /auth/login`, or bootstraps the
    first operator when `app_user` is empty. Session-aware nav across every page (shows the
    operator + Logout, or a Login link).
  - **New entry** (`/ui/new`): HTMX-style forms for vendor, manufacturer, product, serialized
    unit, bulk stock, and system — each serializes to JSON and POSTs to the API.
  - **New purchase** (`/ui/purchases/new`): purchase header + repeatable line-item rows → the
    nested `CreatePurchase` JSON.
  - **Unit detail** (`/ui/units/{id}`): status-change, asset-tag, open-RMA forms + the full
    `unit_event` timeline + a camera-verify link.
  - **System detail** (`/ui/systems/{id}`): members list + add-member, validate, deliver, parts
    sweep, and transfer actions.
- Forms POST JSON to the existing auth-protected routes with the browser's signed session
  cookie, so a logged-in operator's actions just work; anonymous mutations still 401. A single
  shared `cecSubmit` client helper does serialize-and-POST and renders the result inline.
- Tests: `ui_pages_render` extended to cover the new public pages and to render the unit/system
  detail pages against real rows (exercising their joins/enum casts on the live schema).

### Added — 2026-06-27 — Full one-command stack + container smoke gate
- `docker-compose.yml` now wires the **whole stack** — `db` + `extractor` + `api` + `poller` —
  with healthchecks and correct service-to-service wiring. The Rust containers build their
  `DATABASE_URL` from the `POSTGRES_*` parts against the `db` service host (the `.env`
  `localhost` URL is host-run dev only), and `api` reaches the extractor at
  `http://extractor:8900`. New `crates/poller/Dockerfile` (multi-stage release build); the api
  image gains `curl` for its `/readyz` healthcheck.
- `.dockerignore` keeps `.env`/keys/dumps/`target/`/`.git/` out of every image layer (the Rust
  images `COPY . .`) — secret hygiene at the build-context boundary (CLAUDE.md §2).
- CI gains a `compose` job: generates `.env`, `docker compose up -d --build --wait`, then
  smoke-tests `GET /readyz` (proves DB connect + migrations) and the extractor `/health`,
  dumping logs on failure and tearing down with `down -v`. This closes acceptance **gate E**
  (containers) on every push — the GitHub runner has the docker daemon the dev sandbox lacks.

### Added — 2026-06-27 — Phase 3+ build-out (in progress)
- Auth (scope §18): operator accounts (`app_user`, migration 0002), argon2 password hashes,
  and signed session cookies keyed off `SESSION_SECRET`. `POST /auth/bootstrap` (first user),
  `/auth/login`, `/auth/logout`, `/auth/me`, `/auth/users` (authenticated). The data/mutation
  routes are wrapped in `require_auth`; `/health`, `/readyz`, `/auth/*`, and the read-only UI
  stay public. Sits behind the Headscale mesh (enable cookie `Secure` under TLS).
- Receipt→inventory loop (scope §3): `POST /purchases/from-extraction` calls the extractor and
  persists a draft purchase + unresolved line items (storing the raw payload on
  `purchase.raw_extract`) for operator resolution → units. Verified end-to-end against a live
  uvicorn extractor.
- CI now runs the integration suite against a Postgres service (`tests` job) — every exercised
  query is validated against the live schema on each push.
- Operator UI (scope §18 path 1): server-rendered dashboard (`/`) with live counts and
  `/ui/units`, `/ui/systems`, `/ui/purchases` tables (no template-engine dep), plus a
  camera/barcode scan island (`/ui/scan/{unit_id}`) that posts to the verify endpoint
  (native BarcodeDetector; WASM fallback + long-receipt capture are a later pass).
- Python extractor service (scope §11): `services/extractor` FastAPI app — `/health`,
  `/extract` (deterministic template fast-path for known vendors incl. per-line serials, VLM
  fallback stubbed), `/stitch` placeholder. Pure-stdlib `extractor.py` with tests; Dockerfile;
  README. Rust seam `POST /extract-preview` proxies `EXTRACTOR_URL/extract` (502 when down).
- Cross-cutting (scope §12.5/§18/§20): `GET /reorder` (stock at/below reorder point),
  `GET /receiving/reconciliation` (delivered-but-not-received worklist), `GET /export` (full
  no-lock-in JSON snapshot of every table) and `GET /export/units.csv` (units CSV).
- Identity resolution + bundle expansion (scope §3/§15): `POST /line-items/{id}/resolve`
  (map a line to a product, status→confirmed) and `POST /line-items/{id}/expand` (split a
  combo line into child line items by MSRP weight (default) or even, remainder on the last;
  marks the parent `is_bundle`).
- Serial verification + asset tags (scope §13): `POST /units/{id}/verify` (binds a serial
  from the scan or confirms a match; warns on `serial_format_regex` mismatch — never blocks),
  and `POST /units|systems|stock/{id}/asset-tag` (assigns an internal CEC-* tag, idempotent,
  returns a Code128 ZPL label payload).
- cec.direct seam (scope §19): `GET /availability` (in-stock serialized per product + bulk
  qty), `POST /units/{id}/reserve` (in_stock→reserved) and `POST /units/{id}/consume`
  (reserved/in_stock→installed, attach to a System whose build_id references the build),
  guarded transitions, event-logged.
- RMA lifecycle (scope §7): `POST /units/{id}/rma` opens a case (derives execution mode /
  proof source / custody from ownership), `GET/PATCH /rma/{id}`, `GET /rma`,
  `POST /rma/{id}/proof-package` (bundles receipt + serial + warranty terms, stored on the
  case + object store), and `POST /rma/{id}/replacement` (new `rma_replacement` unit linked to
  its predecessor, refurb→refurb class, replacement-term mfr warranty, predecessor retired,
  inherited system re-validates). New RMA domain enums.
- Parts sweep + warranty transfer (scope §6.5): `POST /systems/{id}/sweep` scans+reconciles
  members (matched/missing/unexpected_extra → clean/discrepancies; clean re-validates),
  `POST /systems/{id}/transfer` moves a delivered system to a new owner gated on a clean
  sweep, with per-part manufacturer transferability (carried vs void_non_transferable),
  recorded SystemTransfer + transfer/owner_change events.
- Systems + delivery (scope §6.1/6.2): System CRUD, membership (`/systems/{id}/members`, a
  change invalidates), the validation primitive (`/validate` — a passing EOL/post-change
  validation restores `validated`), and `POST /systems/{id}/deliver` (shop→customer, stamps
  `delivery_datetime`, starts the per-unit CEC clock with refurb-vs-full class, logs
  deliver/owner_change + an EOL SystemValidation; requires the system validated). The unit
  response now includes the mfr/CEC warranty expiry fields.
- Trade-in & opening-balance intake (scope §8/§9): `POST /trade-ins` and `POST /opening-balance`
  resolve units to `owner=shop` and set RMA readiness from the proof situation (provided →
  pending; will-send → `awaiting_proof_from_customer`; lacks/none/unknown-origin →
  `no_proof_of_purchase`); opening-balance rides a synthetic `opening_balance` purchase.
- Warranty engine + RMA readiness (scope §5): `crates/api/src/warranty.rs` (pure, unit-tested)
  computes both warranty clocks, `rma_eligible` + block reasons, and `cec_warranty_active`.
  `CecWarrantyPolicy` CRUD (`/warranty-policies`), `POST /units/{id}/recompute-warranty`, and
  `GET /units/{id}/warranty` (two-clock view with remaining days).

### Added — 2026-06-27 — Phase 1 (part): landed cost + order tracking
- Landed-cost allocation (scope §14): `POST /purchases/{id}/allocate-costs` spreads order-level
  shipping + tax − discount across lines and writes per-line `allocated_landed_cost` and
  per-unit `unit_cost` (logging a `note` event per unit). Pure allocator with unit tests.
- Shipment tracking (scope §12): new `crates/tracking` crate (a `CarrierProvider` trait with
  `none`/`mock` providers + the poll engine), shipment endpoints (`POST /purchases/{id}/shipments`,
  `GET /shipments`, `GET /shipments/{id}`, `POST /shipments/{id}/poll`), and a real
  `crates/poller` worker that polls active shipments and stops on delivery.

### Added — 2026-06-27 — Phase 0 manual-entry CRUD
- Catalog/reference endpoints: vendors, manufacturers, products.
- Purchases with line items (created in one transaction), receipt-file upload to the
  filesystem object store (`POST /purchases/{id}/receipt`, appended to `receipt_files`).
- Serialized units with `unit_event` logging on every mutation: creation writes `intake`,
  `PATCH /units/{id}/status` writes `status_change`; `GET /units/{id}/events` returns the
  timeline (scope §16).
- Bulk stock create + guarded quantity adjustment (`POST /stock/{id}/adjust`).
- The api crate is now lib+bin (`src/lib.rs`); new domain enums in `crates/domain`; money via
  `rust_decimal::Decimal`; an integration test (`crates/api/tests/api.rs`) that drives the full
  flow against a live DB (self-skips without `DATABASE_URL`, so CI stays DB-free).

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

### Fixed — 2026-06-27
- CI `secret-scan` job: pass `GITHUB_TOKEN` to `gitleaks/gitleaks-action@v2`. The action now
  requires it to scan `pull_request` events; without it the gitleaks job failed on PR #1 (a
  config error, not a detected secret — the log confirmed "No license key is required").

### Notes
- Acceptance gates A–D verified in this environment (the DB was a native Postgres 16 cluster
  rather than `docker compose up -d db`, since the sandbox has no docker daemon — deviation
  V-001 in `docs/DECISIONS.md`). Gate E (containers) is ⛔ BLOCKED — no docker daemon. Gate F
  is ◐ PARTIAL — the commit succeeded with `.env` unstaged/ignored, but the local gitleaks run
  is outstanding (deviation V-002; the CI `secret-scan` job is the active backstop). Full,
  authoritative gate table: `docs/HANDOFF.md` entry [2026-06-27].
