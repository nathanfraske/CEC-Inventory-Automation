# TODO — work queue

> Last updated: 2026-06-27 · Dating + tombstoning rules: `CLAUDE.md` §3.
> Open items are dated on creation. Completed/obsolete items move to "Done / Tombstoned"
> with a dated tag — never silently deleted.

## Open — close out Phase 0

- [ ] **[2026-06-27] Close gate E (containers).** On a host with the Docker daemon, run
  `docker compose up -d --build` and verify `curl localhost:8080/readyz` → `{"db":"up"}`
  against the containerized DB. ⛔ BLOCKED [2026-06-27] — no docker daemon in the web sandbox.
- [ ] **[2026-06-27] Close gate F (secret scan) locally.** Install gitleaks and run
  `gitleaks detect --source . --redact` (or `just scan`); confirm zero leaks. CI already runs
  it on push, but a clean local run fully closes the runbook gate.
  ◐ PARTIAL [2026-06-27] — commit succeeded and `.env` is unstaged/ignored (two of the three
  gate conditions met); only the local gitleaks run is outstanding. Local install was denied in
  this environment; the CI `secret-scan` job is the active backstop until then.
- [ ] **[2026-06-27] Verify CI is green on first push.** Confirm the `secret-scan` and `rust`
  jobs pass (`fmt --check`, `clippy -D warnings`, `build` with `SQLX_OFFLINE=true`).

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
- [ ] **[2026-06-27]** Landed-cost allocation (scope §14, default weight-by-line-total / INV-OQ-20).
- [ ] **[2026-06-27]** Shipment capture + the polling worker: fill `crates/poller` per scope §12
  (load active shipments, poll provider, write `shipment_event`, stop on delivery). Carrier
  choice is INV-OQ-30.

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
