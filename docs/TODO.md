# TODO — work queue

> Last updated: 2026-06-28 · Dating + tombstoning rules: `CLAUDE.md` §3.
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
- [x] **Wire the real receipt vision to the local broker.** ✅ DONE [2026-06-27] — added an
  `openai` vision backend in `services/extractor/vision.py` (OpenAI `/chat/completions`, data-URI
  image), wired `EXTRACTOR_VLM_*` + `extra_hosts: host.docker.internal:host-gateway` through
  compose, `.env` `openai` + `cec-vision-judge` (Qwen3-VL-32B). Made it **async** (D-021):
  `from-image-async` + job poll + `/extract/vlm-status`, UI shows warming-vs-warm, with a
  keep-warm script + systemd timer. Live test: receipt image → broker → Qwen3-VL → correct §11.4
  JSON (vendor/lines/serials/totals), `engine=vlm_openai`, images stay on-box (§11.2). D-020/D-021.
  REMAINING: OpenCV long-receipt `/stitch` (still a placeholder); decide whether to enable the
  keep-warm timer (holds ~21 GB VRAM).
- ✅ DONE [2026-06-27] **Backups (Stage 5 complete).** DONE: validated on the box — age-encrypted DB + object-store
  archives via `db_backup.sh`; **restore drill PASSED** (21 tables) via `restore_drill.sh`; scripts
  are container-aware (no host pg tools / no sudo needed; `scripts/_pglib.sh`, D-022).
  ✅ SCHEDULE + OFFSITE DONE [2026-06-27]: `cec-backup.timer` installed + enabled (nightly 02:30 +
  jitter, `Persistent=true`; box-specific unit — `User=nathan`, real checkout, `age` on PATH);
  `scripts/db_backup.sh` now replicates each run's encrypted `*.age` to
  `BACKUP_OFFSITE_DIR=/mnt/c/CEC-Backups` (Windows drive = separate failure domain). Validated via
  the systemd unit (`Result=success`); the **offsite** copy proven restorable (decrypts → 21 tables).
  ✅ KEY DR DONE [2026-06-27]: owner deemed the box secure → the age private key is also replicated
  to `/mnt/c/CEC-Backups/backup-age.key`, kept in sync by `BACKUP_OFFSITE_INCLUDE_KEY=1`; `/mnt/c`
  alone proven to fully restore (decrypt with the offsite key → 21 tables). See
  SECRETS-AND-DATABASE.md §5.1 + HANDOFF [2026-06-27] (backups-schedule).
- [ ] **[2026-06-27] Enable the receipt-vision keep-warm timer** (implement later): install
  `scripts/systemd/cec-vlm-keepwarm.{service,timer}` (needs sudo) so `cec-vision-judge` stays
  resident and operators skip the ~2-3 min cold load. Holds ~21 GB VRAM — enable only when the GPU
  isn't needed for other CEC tools.

## Open — close out Phase 0

- [x] **[2026-06-27] Close gate E (containers).** ✅ DONE [2026-06-27] — the CI `compose` job
  runs `docker compose up -d --build --wait` for the full stack (db+extractor+api+poller) on
  the GitHub runner and smoke-tests `/readyz` + extractor `/health` on every push. The local
  web sandbox still has no docker daemon (deviation V-001), but the container path is now
  verified on CI rather than blocked. `docker compose config` validates locally.
- ✅ DONE [2026-06-27] **Close gate F (secret scan) locally.** Superseded by the "Now on a compute
  box" gate-F item above — gitleaks 8.30.1 installed, `gitleaks detect --source .` clean across 47
  commits, V-002 ended. This Phase-0-section copy held stale web-sandbox state ("local install
  denied"); tombstoned 2026-06-27 after the docs↔code reconciliation confirmed it done.
- [ ] **[2026-06-27] Verify CI is green on first push.** Confirm the `secret-scan` and `rust`
  jobs pass (`fmt --check`, `clippy -D warnings`, `build` with `SQLX_OFFLINE=true`).
  ✅ DONE [2026-06-27] — green on head `81960f6` after the gitleaks `GITHUB_TOKEN` fix.
- [ ] **[2026-06-27] Minimal operator UI for the Phase 0 CRUD.** HTMX + server-render (scope
  §18 path 1) over the existing endpoints: manual purchase/unit/bulk entry, receipt upload, the
  unit event timeline. The API + JSON exist; this is the "usable day one" front end.
- [ ] **[2026-06-27] Migrate to compile-time-checked SQLx queries + commit `.sqlx/`.** Convert
  the runtime `query`/`query_as` calls to `query!`/`query_as!`, run `just prepare`, commit the
  cache (scope INV-OQ-1; see `docs/DECISIONS.md` D-010).
- ✅ DONE [2026-06-27] **Run the integration test in CI.** Verified done in the docs↔code
  reconciliation: `.github/workflows/ci.yml` has a dedicated `tests` job (postgres:16 service +
  migrations + `cargo test --workspace`), so `crates/api/tests/api.rs` executes on every push — it
  no longer self-skips. (The header comment in `crates/api/tests/api.rs` still claims CI is DB-free;
  fix that stale comment next time the file is touched.)

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
  containers + cap_drop/limits; `cargo audit`/`pip-audit`/dependabot. ALSO DONE [2026-06-27]
  (cheap-wins batch): policy-lookup uniqueness (migration 0006); money f64→string in the extractor
  path (D-023); argon2 param pinning; read-only container FS (+tmpfs); vision egress audit + size
  cap. AND [2026-06-27] backups scheduled + encrypted + offsite + restore drill (Stage 5). CSRF
  multipart is already covered by the same-origin guard (`auth.rs`), not a separate token. OUTSTANDING:
  server-side session revocation; per-IP/persistent login limiting; finer RBAC + `/export`
  admin-gating + session-derived actor; image/Actions digest pinning + flip audit to blocking;
  WAL/PITR + a true off-machine backup target; cross-table asset-tag uniqueness.

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

- ✅ DONE [2026-06-28] API integration docs completed as a standalone contract: field-level
  request/response schemas for all 73 endpoints in `docs/API.md` (§ Endpoint schemas) + the 3 async
  endpoints + 202/500 status codes; `docs/INTEGRATION.md` token-lifecycle/limits/conventions; and
  the `GET /manufacturers/{id}` mount fix (+ test). → HANDOFF [2026-06-28] (api-docs). New small
  follow-up: switch `phase2_verify_and_asset_tags` hardcoded serials to the `sn()` helper; consider
  a dedicated test DB so local `cargo test` doesn't write into the dev DB.

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
