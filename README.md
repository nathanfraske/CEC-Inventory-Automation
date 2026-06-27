# CEC Inventory

Custom shop inventory system. Receipt to inventoried parts with serials, prices,
two-layer warranty, RMA lifecycle, system re-validation, and order tracking.

Source of truth for the design: `docs/CEC-Inventory-System-Scope.md`.

## API
The backend is a plain JSON-over-HTTP API.
- **`docs/API.md`** — the full endpoint reference (every route, method, and auth level).
- **`docs/INTEGRATION.md`** — how an external app integrates: bearer-token auth, the cec.direct
  availability/reserve/consume seam, receipt push, export, and conventions, with a curl
  walkthrough.

## Spin-up
Agents and humans: follow `AGENT_RUNBOOK.md`. It is ordered and has acceptance
gates at each phase.

## Secrets and the database
Read `SECRETS-AND-DATABASE.md` before the first commit. Short version: real
credentials live only in a gitignored `.env`, the database data lives in a Docker
volume (never in the repo), and CI fails on any committed secret.

## Layout
- `crates/api`     Rust + Axum backend and HTMX UI
- `crates/poller`  shipment tracking worker
- `crates/domain`  shared domain types
- `migrations`     SQLx SQL migrations
- `services/extractor`  Python receipt stitching + extraction service (inference box)
- `docs`           the scope spec
- `scripts`        secret generation, backup, restore

License: Apache-2.0.
