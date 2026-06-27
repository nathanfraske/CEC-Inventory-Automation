# Changelog

> Last updated: 2026-06-27

All notable changes to this repository are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/); dates are ISO-8601. See `CLAUDE.md` §3 for the
dating + tombstoning conventions that govern the memory documents.

## [Unreleased]

### Added — 2026-06-27 — Phase 3+ build-out (in progress)
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
