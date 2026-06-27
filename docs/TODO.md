# TODO — work queue

> Last updated: 2026-06-27 · Dating + tombstoning rules: `CLAUDE.md` §3.
> Open items are dated on creation. Completed/obsolete items move to "Done / Tombstoned"
> with a dated tag — never silently deleted.

## Open — close out Phase 0

- [x] **[2026-06-27] Close gate E (containers).** ✅ DONE [2026-06-27] — the CI `compose` job
  runs `docker compose up -d --build --wait` for the full stack (db+extractor+api+poller) on
  the GitHub runner and smoke-tests `/readyz` + extractor `/health` on every push. The local
  web sandbox still has no docker daemon (deviation V-001), but the container path is now
  verified on CI rather than blocked. `docker compose config` validates locally.
- [ ] **[2026-06-27] Close gate F (secret scan) locally.** Install gitleaks and run
  `gitleaks detect --source . --redact` (or `just scan`); confirm zero leaks. CI already runs
  it on push, but a clean local run fully closes the runbook gate.
  ◐ PARTIAL [2026-06-27] — commit succeeded and `.env` is unstaged/ignored (two of the three
  gate conditions met); only the local gitleaks run is outstanding. Local install was denied in
  this environment; the CI `secret-scan` job is the active backstop until then.
- [ ] **[2026-06-27] Verify CI is green on first push.** Confirm the `secret-scan` and `rust`
  jobs pass (`fmt --check`, `clippy -D warnings`, `build` with `SQLX_OFFLINE=true`).
  ✅ DONE [2026-06-27] — green on head `81960f6` after the gitleaks `GITHUB_TOKEN` fix.
- [ ] **[2026-06-27] Minimal operator UI for the Phase 0 CRUD.** HTMX + server-render (scope
  §18 path 1) over the existing endpoints: manual purchase/unit/bulk entry, receipt upload, the
  unit event timeline. The API + JSON exist; this is the "usable day one" front end.
- [ ] **[2026-06-27] Migrate to compile-time-checked SQLx queries + commit `.sqlx/`.** Convert
  the runtime `query`/`query_as` calls to `query!`/`query_as!`, run `just prepare`, commit the
  cache (scope INV-OQ-1; see `docs/DECISIONS.md` D-010).
- [ ] **[2026-06-27] Run the integration test in CI.** Add a Postgres service to the `rust`
  job, run migrations, and `cargo test` so `crates/api/tests/api.rs` executes in CI (today it
  self-skips without `DATABASE_URL`).

## Open — Phase 1 (receipt capture, extraction, cost, order tracking) — scope §20

- [ ] **[2026-06-27]** Wire object storage behind the API (filesystem default, `STORAGE_FS_ROOT`;
  MinIO is the drop-in per INV-OQ-2).
- [ ] **[2026-06-27]** Stand up the Python extractor service (`services/extractor`, FastAPI):
  OpenCV stitching pre-step, `invoice2data` template fast-path, VLM fallback. Emits the
  scope §11.4 JSON schema.
- [ ] **[2026-06-27]** Whole-page web capture + guided overlapping capture for long receipts
  (scope §10).
- [ ] **[2026-06-27]** Receipt → auto-populated line items with operator confirmation; identity
  resolution with bundle expansion (scope §3, §15).
- [x] **[2026-06-27]** Landed-cost allocation (scope §14, default weight-by-line-total / INV-OQ-20).
  ✅ DONE [2026-06-27] — `POST /purchases/{id}/allocate-costs` + `crates/api/src/costing.rs`.
- [x] **[2026-06-27]** Shipment capture + the polling worker: fill `crates/poller` per scope §12
  (load active shipments, poll provider, write `shipment_event`, stop on delivery).
  ✅ DONE [2026-06-27] — `crates/tracking` (provider trait + mock/none + poll engine), shipment
  endpoints, and the real poller binary. Real carrier APIs / aggregator remain open (INV-OQ-30).
- [ ] **[2026-06-27]** Wire a real carrier provider (USPS/UPS/FedEx/DHL direct, or an
  aggregator) behind the `CarrierProvider` trait; today only `none`/`mock` exist (INV-OQ-30).

## Open — hardening follow-ups (backend for Phases 1–5 is done; see HANDOFF [2026-06-27])

- ◐ PARTIAL **[2026-06-27]** SQLx hardening (D-010). DONE: a CI `tests` job runs the full
  integration suite against a Postgres service, validating every exercised query against the
  live schema on each push. OUTSTANDING: convert runtime `query`/`query_as` to compile-time
  `query!`/`query_as!` + commit `.sqlx/` (a large rewrite — the current SQL is built with
  `format!` column-list consts that macros can't take; the CI run covers the correctness gap
  meanwhile).
- [ ] **[2026-06-27]** Extractor: wire a real VLM + OpenCV stitching on the inference box
  (deterministic template path + Rust seam are done). (§10/§11)
- [ ] **[2026-06-27]** Real carrier provider(s) behind the `CarrierProvider` trait (USPS/UPS/
  FedEx/DHL or an aggregator); only `none`/`mock` exist. (INV-OQ-30)
- [ ] **[2026-06-27]** UI: WASM scan fallback (Safari/iOS), guided long-receipt capture, and
  HTMX-driven manual-entry forms over the JSON API. (§10/§13/§18)

## Done — Phases 1–5 backend (✅ DONE [2026-06-27], details in HANDOFF + CHANGELOG)

- ✅ §5 warranty engine + RMA readiness · ✅ §8/§9 trade-in/opening-balance · ✅ §6.1/6.2 systems
  + delivery · ✅ §6.4/6.5 sweep + transfer · ✅ §7 RMA lifecycle · ✅ §19 cec.direct seam ·
  ✅ §13 serial verify + asset tags · ✅ §3/§15 identity resolution + bundles · ✅ §12.5/§18/§20
  reorder/reconciliation/export · ✅ §11 extractor (deterministic path) · ✅ §18 server-rendered UI.

## Open — later phases (track here as they start; full detail in scope §20)

- [ ] **[2026-06-27]** Phase 2 — browser scan loop (native + WASM), serial→unit binding +
  verification pass, serial-format validation, asset-tag printing, opening-balance intake.
- [ ] **[2026-06-27]** Phase 3 — ownership-aware readiness, delivery flow (starts the CEC clock),
  two-warranty display, RMA lifecycle (three execution modes, proof-of-purchase package).
- [ ] **[2026-06-27]** Phase 4 — system validation state, re-validation flow, parts sweep,
  warranty-transfer path gated on a clean sweep.
- [ ] **[2026-06-27]** Phase 5 — cec.direct seam (availability + reserve/consume, `build_id`
  linkage, field-RMA provenance recovery).
- [ ] **[2026-06-27]** Cross-cutting (slot anytime): reorder workflow behind `reorder_point`,
  receiving reconciliation (scope §12.5), backup/DR + CSV/JSON export.

## Convention reminders (keep these true as work lands)

- [ ] Add a new numbered migration per schema change; never edit an applied migration.
- [ ] Run `just prepare` (writes/commits `.sqlx/`) after adding any compile-time-checked query.
- [ ] Write a `unit_event` row on every unit mutation (scope §16) — wire it as each feature lands.

---

## Done / Tombstoned

- ✅ DONE [2026-06-27] Lay down the Phase 0 seed scaffold and pass gates A–D. → `docs/HANDOFF.md`
  entry [2026-06-27].
- ✅ DONE [2026-06-27] Set up configuration hooks (git pre-commit secret-scan + Claude Code
  SessionStart) and the memory documents (CLAUDE.md, HANDOFF, TODO, DECISIONS, CHANGELOG) with
  the dating/tombstoning convention. → `CLAUDE.md` §3–§4.
- ✅ DONE [2026-06-27] Phase 0 manual-entry CRUD: purchases + line items, serialized units,
  bulk stock, receipt-file upload to the object store, and `unit_event` logging on every unit
  mutation, with an integration test. → `docs/HANDOFF.md` entry [2026-06-27] (CRUD).
- ✅ DONE [2026-06-27] Phase 1 (part): landed-cost allocation (`/allocate-costs`) and shipment
  capture + polling worker (`crates/tracking`, shipment endpoints, real poller). →
  `docs/HANDOFF.md` entry [2026-06-27] (Phase 1 part).
