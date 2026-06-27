# TODO — work queue

> Last updated: 2026-06-27 · Dating + tombstoning rules: `CLAUDE.md` §3.
> Open items are dated on creation. Completed/obsolete items move to "Done / Tombstoned"
> with a dated tag — never silently deleted.

## Now on a compute box [2026-06-27] — first things to do

The project moved from the web sandbox (no docker daemon / no gitleaks) to a real box; PR #1 is
merged to `main`. Do these to close the last sandbox gaps (full steps: `CLAUDE.md` §8):
- [x] **Close gate E locally.** ✅ DONE [2026-06-27] — full stack built + came up healthy
  (db/extractor/api/poller); `/readyz`=`{"db":"up"}`, extractor `/health` ok. V-001 ended
  (D-019: api published on 8081 because the broker owns 8080).
- [x] **Close gate F locally.** ✅ DONE [2026-06-27] — gitleaks 8.30.1 installed; `gitleaks
  detect --source .` clean (no leaks, 47 commits). V-002 ended.
- [ ] **Wire the real receipt vision to the local broker (chosen path):** the broker is
  **OpenAI-shaped** (`/v1/chat/completions`; vision seat `cec-worker-vision` / Qwen3-VL) but
  `vision.py` is **Anthropic-Messages-shaped** — add an OpenAI vision backend in `vision.py`,
  pass `ANTHROPIC_BASE_URL`/broker vars + `extra_hosts: host.docker.internal:host-gateway` to the
  extractor in compose, then test `POST /extract-image`. Keeps receipt images on-box (§11.2).
- [ ] **Stand up scheduled backups:** install `scripts/systemd/cec-backup.{service,timer}`, set
  `BACKUP_AGE_RECIPIENT` (encryption) + an offsite target; verify with `scripts/restore_drill.sh`.

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

## Open — audit remediation backlog (panels ran 2026-06-27; full list in docs/AUDIT-2026-06-27.md)

- ◐ PARTIAL **[2026-06-27]** Audit panels (security + data-integrity/backups), worked in 4
  batches — full status in `docs/AUDIT-2026-06-27.md`. DONE: object-store backup (Critical);
  `SESSION_SECRET` fail-closed; body-size limits; reserve/consume + rma_update/unit_label
  transactional fixes; regex size limit; extraction validation; password floor 12; constant-time
  login; **serial-number uniqueness (globally unique, D-017)** + asset-tag uniqueness +
  append-only triggers (migration 0003); system-gating TOCTOU locks; status-transition matrix;
  `/export` completeness; session TTL + login throttle + RBAC (migration 0004); non-root
  containers + cap_drop/limits; `cargo audit`/`pip-audit`/dependabot. OUTSTANDING: server-side
  session revocation; per-IP/persistent login limiting; finer RBAC + `/export` admin-gating +
  session-derived actor; CSRF tokens (multipart); read-only container FS + db cap hardening;
  image/Actions digest pinning + flip audit to blocking; backup automation/encryption/offsite +
  restore drill + WAL/PITR; money f64→string in the extractor path; cross-table asset-tag
  uniqueness; policy-lookup unique constraints; argon2 param pinning; vision egress audit/size cap.

## Open — hardening follow-ups (backend for Phases 1–5 is done; see HANDOFF [2026-06-27])

- ◐ PARTIAL **[2026-06-27]** SQLx hardening (D-010). DONE: a CI `tests` job runs the full
  integration suite against a Postgres service, validating every exercised query against the
  live schema on each push. OUTSTANDING: convert runtime `query`/`query_as` to compile-time
  `query!`/`query_as!` + commit `.sqlx/` (a large rewrite — the current SQL is built with
  `format!` column-list consts that macros can't take; the CI run covers the correctness gap
  meanwhile).
- ◐ PARTIAL **[2026-06-27]** Extractor vision (§10/§11.2). DONE: an interim image-vision path —
  `vision.py` with a `claude` backend (Anthropic Messages API) behind `EXTRACTOR_VLM_BACKEND`,
  `POST /extract-image`, the Rust `from-image` (multipart) + `from-payload` seams, and a UI
  receipt block. OUTSTANDING: the **local** VLM (Qwen2.5-VL etc.) on the inference box so images
  stay on-prem, and OpenCV long-receipt stitching (`/stitch` is still a placeholder).
- [ ] **[2026-06-27]** Real carrier provider(s) behind the `CarrierProvider` trait (USPS/UPS/
  FedEx/DHL or an aggregator); only `none`/`mock` exist. (INV-OQ-30)
- ◐ PARTIAL **[2026-06-27]** UI build-out (§10/§13/§18). DONE: login/first-run page +
  session-aware nav; manual-entry forms (vendor/manufacturer/product/unit/stock/system);
  new-purchase form with line items; unit detail (status/asset-tag/open-RMA + event timeline);
  system detail (members/validate/deliver/sweep/transfer). All POST JSON to the auth-protected
  API with the session cookie (D-014). OUTSTANDING: WASM scan fallback (Safari/iOS) and guided
  long-receipt capture (need a device + the inference box); line-item resolve/expand and the
  receipt-upload control are not yet surfaced in the UI (the endpoints exist).

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

- ✅ DONE [2026-06-27] Add mechanical §3-protocol enforcement hooks: Claude Code **Stop**
  (`.claude/hooks/enforce-doc-compliance.sh`, blocks code-without-docs) + **PostToolUse**
  date-check (`.claude/hooks/check-doc-dating.sh`) + SessionStart protocol surfacing. →
  `CLAUDE.md` §4, `docs/HANDOFF.md` entry [2026-06-27] (hooks).
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
