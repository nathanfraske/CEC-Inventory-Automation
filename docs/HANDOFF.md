# HANDOFF — current state of the build

> Last updated: 2026-06-28 · Append a new dated entry per session; never overwrite history.
> Read this with `docs/TODO.md` (work queue) and `CLAUDE.md` (operating contract).
> Dating + tombstoning rules: `CLAUDE.md` §3.

---

## Entry [2026-06-28] — API docs completed as a standalone integration contract + GET /manufacturers/{id} fix (agent: claude, api-docs)

A verification fan-out first confirmed the integration posture: **every capability is already
bearer-reachable** (86 routes; 75 authed all accept `Authorization: Bearer cec_pat_…` via one
`resolve_role` path; CSRF only gates cookie callers; zero UI/cookie-only features). So this was a
**docs + one-route-fix** task, not a capability gap.

- **`docs/API.md`** gained a **§ Endpoint schemas** section: field-level request + response shapes
  for all 73 endpoints (incl. auth/token mgmt), extracted from the handler source via an 8-way
  fan-out and spot-verified against the code (catalog/extractor sections checked by hand). Also
  added the 3 async extraction endpoints to the catalog tables and the `202`/`500` status codes +
  the FK-`400` vs unique-`409` distinction.
- **`docs/INTEGRATION.md`**: token lifecycle (no expiry/scopes, rotate operationally), body-size
  limits, ISO-8601 UTC timestamps, query params, status-code table.
- **Bug fix:** `GET /manufacturers/{id}` was never mounted (vendors + products had GET-one;
  manufacturers didn't — the audit's "handler exists but unmounted" was wrong; it didn't exist).
  Added `catalog::get_manufacturer` (uses `MANUFACTURER_COLS` for the enum text casts) + the route +
  a `manufacturer_get_one_and_404` integration test.

Validation: `cargo fmt`/`clippy -D warnings` clean; full suite **10 unit + 20 integration green**.
Note: one local-only flake first appeared — `phase2_verify_and_asset_tags` hardcodes serial
`GPU-1234A` (not the idempotent `sn()` helper), so it collides on a re-run against the persistent
dev DB (globally-unique serials, D-017); freed the parked serial and it passed. CI uses a fresh DB,
so it's unaffected. **Follow-up (small):** switch phase2's hardcoded serials to `sn()`. Also note:
running the integration suite locally writes test rows into the live dev DB — consider a dedicated
test database. Merged to main.

---

## Entry [2026-06-27] — Cheap-wins security/hardening batch (5 audit items), validated + merged to main (agent: claude, cheap-wins)

Shipped the verified "cheap-wins" batch from the reconciliation (one PR's worth, 5 audit items):

1. **Policy-lookup uniqueness** — new migration `0006_policy_unique`: partial unique indexes on
   `vendor_return_policy (vendor_id, category)` + `cec_warranty_policy (warranty_class, category)`,
   plus single-default (NULL-category) indexes per key (a plain UNIQUE treats NULLs as distinct and
   would let two defaults through). Touched `lib.rs` so `sqlx::migrate!` re-embeds. Applied to the
   live DB on boot; duplicate-default insert now **rejected** (verified via psql).
2. **Lossless receipt money (D-023)** — `extractor.py` emits money as exact 2-decimal **strings**
   (`_money_str`, both template + VLM paths); `extractor.rs::money()` parses a string straight to
   `Decimal` (numeric(12,2)) with a **numeric fallback** so `from-payload` callers sending numbers
   still work. Closes the f64-drift gap on receipt totals (the shop's #1 accuracy priority).
3. **Argon2 params pinned** — `auth.rs` `argon2()` helper (Argon2id, 64 MiB / 3 / 4) replaces
   `Argon2::default()`. Existing hashes still verify (PHC params are read from each stored hash).
4. **Read-only container FS** — `read_only: true` + `tmpfs:[/tmp]` on api/poller/extractor (db
   stays writable; api still writes receipts to the `objects` volume mount). All three boot healthy.
5. **Vision egress guard** — `vision.py` caps image size before egress
   (`EXTRACTOR_VLM_MAX_IMAGE_BYTES`, 16 MiB default → 413 over-cap, new `ImageTooLargeError` → 413 in
   `app.py`) and logs each real outbound vision call (backend/model/dest/bytes).

**Validation (all green):** `cargo fmt`/`clippy -D warnings` clean; full Rust suite **10 unit + 19
integration** pass (migrations incl. 0006 ran against the live DB; auth test passed under the new
argon2 params). Python **extractor + vision** tests pass incl. a new oversized-image test. Rebuilt
+ recreated api/extractor/poller — all healthy with `ReadonlyRootfs=true`. **End-to-end:** posted a
real receipt (the owner's tweezers screenshot) through the broker vision path → SIM Supply, Inc.,
one Excelta line, money returned as exact strings (`unit_price "59.40"`, `tax "4.90"`, `total
"64.30"`), `engine vlm_openai`.

**Merged to `main`.** This batch + the backups-schedule entry below + the prior compute-box stack
were merged to `main` and pushed (the 4 stacked PRs land together). Docs updated:
AUDIT-2026-06-27 (5 items flipped to ✅ FIXED + backup-automation offsite noted), DECISIONS (D-023),
CHANGELOG, CLAUDE.md (migration list → 0006), this entry, TODO.

Files: `migrations/0006_policy_unique.sql` (new), `crates/api/src/{lib,extractor,auth}.rs`,
`services/extractor/{extractor,vision,app}.py` + their tests, `docker-compose.yml`,
`.env`/`.env.example`.

Remaining higher-effort security items (next): `/export` admin-gating + session-derived actor;
server-side session revocation + per-IP login limiting; base-image/Actions digest pinning + flip
the supply-chain job to blocking; cross-table asset-tag uniqueness.

---

## Entry [2026-06-27] — Backups now SCHEDULED + replicated offsite; "what's next" reconciled docs↔code (agent: claude, backups-schedule)

Closed the scheduling + offsite halves of Stage 5 (the prior entry left them deferred).

**Scheduled backups — live.** Installed `cec-backup.{service,timer}` to `/etc/systemd/system/` and
`systemctl enable --now`ed the timer. Next run **Sun 2026-06-28 02:34 CDT**; nightly 02:30 +
`RandomizedDelaySec=300`, `Persistent=true` (catches a missed run at next boot). The shipped
template targets `/opt/cec-inventory` + `User=cec`, so I installed a **box-specific** unit instead
(real nested checkout, `User=nathan`/`Group=docker` mirroring `cec-llm-broker.service`, and
`Environment=PATH=/home/nathan/.local/bin:…` so systemd can find `age`). The committed template
under `scripts/systemd/` stays generic/portable. Validated by running the unit (not just the
script): `Result=success`, exit 0, journal shows both encrypted archives written + offsite copy.

**Offsite replication — live.** Added a generic, env-gated step to `scripts/db_backup.sh`: when
`BACKUP_OFFSITE_DIR` is set it copies the run's encrypted `*.age` archives there (same retention).
Set `BACKUP_OFFSITE_DIR=/mnt/c/CEC-Backups` in `.env` (the Windows drive = a different failure
domain than the WSL2 ext4 vdisk). Documented all `BACKUP_*` in `.env.example` (was undocumented).
**Proven recoverable end-to-end:** decrypted the OFFSITE dump with the age key and
`pg_restore --list` enumerated all 21 tables.

**Key DR — closed (owner's call).** The owner deemed the machine secure, so the age private key is
replicated offsite too: copied to `/mnt/c/CEC-Backups/backup-age.key`, made durable via
`BACKUP_OFFSITE_INCLUDE_KEY=1` (the script re-mirrors it each run, so a key rotation stays in sync).
**Proven:** decrypting the offsite archive with the *offsite* key copy restored all 21 tables — so
`/mnt/c` alone fully recovers after a vdisk loss. Tradeoff recorded: this co-locates key+ciphertext
and the key is world-readable on NTFS (acceptable under the "machine is secure" decision; safer
options are `age -p` or a password-manager copy). See SECRETS-AND-DATABASE.md §5.1.

Reverse the schedule with: `sudo systemctl disable --now cec-backup.timer && sudo rm
/etc/systemd/system/cec-backup.{service,timer}`.

**"What's next" reconciliation.** Ran a fan-out workflow (6 read-only verifiers + synthesis) that
checked every open TODO against the actual code/git state. Headline: backups DR was the #1 risk
(done now). Verified-**stale** TODO items to tombstone next pass — (1) "Run the integration test in
CI" is DONE (`ci.yml` has a `tests` job: postgres:16 service + migrations + `cargo test`);
(2) the Phase-0-section "Close gate F locally" duplicate is DONE (gitleaks 8.30.1 clean, V-002
ended); (3) audit "CSRF tokens (multipart)" is already covered by the same-origin guard
(`auth.rs` `same_origin`/`csrf_ok`). Top remaining threads (next sessions): merge PRs #12→#15 to
main; build the `/ui/purchases/{id}` line-item resolve/expand + receipt-upload UI (the receipt→
inventory missing link); a cheap-wins security PR (policy-lookup UNIQUE constraints, money
f64→string in the extractor, argon2 param pinning, read-only container FS, vision egress cap);
then `/export` admin-gating + session-derived actor, digest pinning, and a real carrier provider.

Changed this entry: `scripts/db_backup.sh` (+offsite step), `.env`/`.env.example`, this doc +
TODO + CHANGELOG + CLAUDE.md §5/§8. No Rust/Python changed.

---

## Entry [2026-06-27] — Stage 5: encrypted, container-aware backups validated; scheduling + keep-warm deferred (agent: claude, backups)

Stood up and **proved** backups on the box (CLAUDE.md §8 step 5):
- `scripts/db_backup.sh` → age-**encrypted** DB dump (`*.dump.age`) + receipt object-store archive
  (`*.tar.gz.age`) in `BACKUP_DIR` (`~/cec-backups`), with retention pruning.
- `scripts/restore_drill.sh` → decrypts, restores into a throwaway DB, verified **21 public
  tables — PASSED**.
- **Container-aware (new `scripts/_pglib.sh`):** this box has no host `psql`/`pg_dump`/`pg_restore`
  (and no passwordless sudo to install them) and the data lives in Docker volumes, so the scripts
  now route pg tools through the `db` service and archive/restore receipts via the
  `cec-inventory_objects` named volume when there is no host `STORAGE_FS_ROOT`
  (`CEC_PG_MODE=host|container`; host tools still take precedence). D-022.
- **age** v1.3.1 installed (`~/.local/bin`); keypair at `~/.config/cec/backup-age.key`, recipient
  in the gitignored `.env`. ⚠️ **The private key is NOT in git — back it up offsite or the
  encrypted backups are unrecoverable.**

Also cleaned the DB to a fresh slate: removed the session's test draft purchases + the test
vendor (admin account retained); 0 purchases / 0 vendors now.

Deferred (need sudo / a target), tracked in `docs/TODO.md`:
- Enable the backup **schedule** (`scripts/systemd/cec-backup.{service,timer}`) + an **offsite**
  replication target.
- Enable the receipt-vision **keep-warm** timer (`scripts/systemd/cec-vlm-keepwarm.*`) — holds
  ~21 GB VRAM, so on only when the GPU isn't needed elsewhere.

No Rust/Python changed this entry; only ops scripts + docs.

---

## Entry [2026-06-27] — Async receipt-vision flow + cec-vision-judge seat + keep-warm (agent: claude, async extract)

Made receipt-image extraction **asynchronous** so the operator UI never blocks on a (possibly
cold) vision load and clearly shows **warming vs warm**:
- **API:** `POST /purchases/from-image-async` registers an in-memory job (AppState `vlm_jobs`,
  pruned at 30 min) and returns `202 {job_id}` immediately; a background tokio task resolves the
  warm-state then runs extract + persist. `GET /purchases/from-image-jobs/{id}` polls
  (`status` + `model_warm`); `GET /extract/vlm-status` reports warm state. The extractor gained
  `GET /vlm-status` (reads the broker `/v1/models` running flag). The UI polls and labels
  "Warming…" vs "Extracting…", then opens the draft purchase to confirm (D-021).
- **Vision seat → `cec-vision-judge`** (Qwen3-VL-32B, GPU-resident) per the owner;
  `.env EXTRACTOR_VLM_MODEL=cec-vision-judge`.
- **Keep-warm:** `scripts/vlm_keepwarm.sh` (pings the broker; model read from `.env`) +
  `scripts/systemd/cec-vlm-keepwarm.{service,timer}` (every 20 min, under the broker's 30-min idle
  reap) so the seat stays resident. Opt-in like the backup timer; **NOT yet installed.**
- **Supply-chain:** reqwest switched to `default-features=false` (rustls only) — `openssl-sys` is
  gone from the dependency tree/image (matches sqlx `tls-rustls`; audit supply-chain item).

**Live-validated:** cold submit → immediate 202, job `model_warm:false`, polled through the
~159 s cec-vision-judge cold-load → `ready` with a draft (vendor / 3 lines / per-line serials,
engine `vlm_openai`); warm submit → `model_warm:true`, ~8 s. 29 tests green (10 unit + 19
integration, incl. 2 new async ones), fmt + clippy clean.

Hardening from an adversarial review (6 confirmed findings, all fixed): bounded timeouts on the
api→extractor client (connect 10 s / request 300 s) so a wedged broker **fails** the job instead
of hanging and leaking the task+image; the warm probe moved off the 202 path (truly immediate);
the UI poll gained an 8-min deadline + poll-error surfacing; keep-warm model sourced from `.env`.

Follow-ups: OpenCV `/stitch` still a placeholder; enabling the keep-warm timer (holds ~21 GB VRAM)
is the operator's call; a first cold scan after 30-min idle is ~2-3 min unless the timer runs.

---

## Entry [2026-06-27] — On-box receipt vision wired to the cec-llm-broker (agent: claude, extractor vision)

The receipt-**image** path now runs **on-box**. Added an `openai` backend to
`services/extractor/vision.py` (OpenAI `/chat/completions` with a data-URI `image_url` block),
selected by `EXTRACTOR_VLM_BACKEND=openai`; configured via `EXTRACTOR_VLM_BASE_URL` (the broker),
`EXTRACTOR_VLM_MODEL`, optional `EXTRACTOR_VLM_API_KEY`/`_MAX_TOKENS`/`_TIMEOUT`. Compose now
passes those through and adds `extra_hosts: host.docker.internal:host-gateway` so the container
reaches the broker on the host (`ANTHROPIC_BASE_URL` is passed through now too). Two hermetic unit
tests lock the wire format + the `vlm_openai` engine tag (6 vision tests green).

**Live-validated:** `.env` set to `openai` + `cec-worker-vision`; a generated receipt PNG through
`POST /extract-image` → broker cold-loaded the Qwen3-VL seat (~8.7 GB VRAM, ~75 s cold) → returned
a correct §11.4 object (vendor `MICRO CENTER`, 3 line items with per-line serials, subtotal/tax/
total), `engine=vlm_openai`. Receipt images never leave the box (scope §11.2 satisfied).

Notes / follow-ups:
- Committed default stays `EXTRACTOR_VLM_BACKEND=stub` (hermetic for CI/other boxes); only this
  box's gitignored `.env` selects `openai`.
- **Perf:** the default seat `cec-worker-vision` (Qwen3.6-35B, experts in host RAM via
  `--n-cpu-moe`) is slow on warm inference (a warm request exceeded 120 s). For snappier OCR point
  `EXTRACTOR_VLM_MODEL` at `cec-vision-judge` (Qwen3-VL-32B, GPU-resident) or raise the broker GPU
  budget. Cold extraction proved correctness; latency is a seat-tuning choice.
- **UX:** a cold load blocks the browser→api→extractor call (~75 s+). The api→extractor reqwest
  has no timeout (rides through); consider pre-warming the seat or an async extract+confirm flow.
- Still open: OpenCV long-receipt `/stitch` (placeholder).

---

## Entry [2026-06-27] — Stack stood up + validated on the compute box; gates E & F closed (agent: claude, compute-box bring-up)

First real boot of the full `docker compose` stack on the box (it had only ever run in CI).
**All four services healthy:** db, extractor (`vlm_backend: stub`), api, poller. `/readyz` →
`{"db":"up"}` (migrations 0001–0005 auto-applied). Validated end-to-end: `POST /auth/bootstrap`
(first admin) → login (signed cookie) → authenticated reads → a CSRF-guarded write (created a
vendor) → minted + used a `cec_pat_` bearer token.

- **Gate E CLOSED / V-001 ended** — the committed `docker compose up -d --build` path works on a
  real daemon, not just CI.
- **Gate F CLOSED / V-002 ended** — installed gitleaks 8.30.1; `gitleaks detect --source .` =
  **no leaks** across 47 commits (the only `--no-git` hits are the generated secrets in the
  gitignored `.env`, by design).
- **Port conflict resolved:** the `cec-llm-broker` permanently owns `127.0.0.1:8080`, so the api
  could not bind it. Made the published host port configurable (`API_PUBLISH_PORT`, default 8080;
  D-019); this box runs the api on **8081** (container-internal stays 8080). Admin/token/UI are at
  `http://127.0.0.1:8081`.
- **Storage note:** this distro's vhdx lives on `E:\wsl\Ubuntu-24.04`, so Docker grows on **E:**
  (~328 GB free, 92% full alongside 3.4 TB of AI models). Watch image/build-cache growth; the
  vhdx does not auto-shrink.

Next (per TODO): wire the extractor to the local broker — but the broker is **OpenAI-shaped**
(`/v1/chat/completions`, vision seat `cec-worker-vision` / Qwen3-VL), while `vision.py` is
**Anthropic-Messages-shaped**, so this needs a new OpenAI vision backend in `vision.py`
(+ `ANTHROPIC_BASE_URL`/broker passthrough + `extra_hosts` in compose), not a config flip. Then
scheduled backups.

---

## Entry [2026-06-27] — Live on the compute box; §3 enforcement hooks added (agent: claude, compute-box onboarding)

`main` pulled onto the real compute box (RTX 5090 32 GB · Core Ultra 7 265K, 18c · ~172 GB RAM ·
~923 GB free root). Docker 29.5.3 + Compose v5.1.4 present; host Rust 1.96.0 lives in `~/.cargo`
but is OFF the WSL `PATH` (only the Windows cargo is on it), so use the **Docker build path** —
the api/poller Dockerfiles compile the binaries inside `rust:1-bookworm`, no host toolchain
needed. The full container stack has still **NOT** been booted here and **no `.env` exists yet**;
first-run steps are unchanged (CLAUDE.md §8 / the TODO "Now on a compute box" section). Gates
E/F and deviations V-001/V-002 remain open pending that first boot + a local gitleaks scan.

Added mechanical enforcement of the §3 memory-doc protocol (see CLAUDE.md §4 / CHANGELOG): a
**Stop** hook (`.claude/hooks/enforce-doc-compliance.sh`) that blocks finishing when source/ops
files change without a memory-doc update, and a **PostToolUse** hook
(`.claude/hooks/check-doc-dating.sh`) that date-checks memory-doc edits; SessionStart now also
surfaces the protocol + open-TODO count. All three were tested (allow/block/date-check paths)
before landing.

**Box note for the headline extractor integration:** a local **`cec-llm-broker`** is already
running — OpenAI-style `/v1/models` on `127.0.0.1:8080`, managing named models incl. vision ones
(`cec-worker-vision`, `cec-vision-judge`, a Windows-brokered `cec-worker-vision-win`) and a
currently-loaded `deepseek-v4-flash`; a live-run dashboard is on `:8090`. The extractor's
`services/extractor/vision.py` is hard-coded to the **Anthropic Messages** wire format
(`POST /v1/messages`, `x-api-key`, `anthropic-version`), so pointing it at the broker is **not a
drop-in** unless the broker exposes an Anthropic-compatible endpoint — otherwise add an
OpenAI-shaped backend in `vision.py`. Also `docker-compose.yml` does **not** yet pass
`ANTHROPIC_BASE_URL` through to the extractor container, so that must be added before any
broker URL takes effect.

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
