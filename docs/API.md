# CEC Inventory — API reference

> Last updated: 2026-06-27 · For the integration walkthrough (auth, the cec.direct seam, a
> curl tutorial), see `docs/INTEGRATION.md`. This file is the endpoint catalog.

All endpoints are JSON-over-HTTP. Money is sent/received as **strings** (`"1599.00"`); IDs are
UUIDs; enums are `snake_case`; errors are `{"error":"…"}`. Auth column:

- **Public** — no credential.
- **Operator** — a valid session cookie *or* `Authorization: Bearer cec_pat_…` token.
- **Admin** — same, but the principal's role must be `admin`.

A unit mutation always appends to the immutable event log (`GET /units/{id}/events`).

## Health & auth

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/health` | Public | Liveness → `ok`. |
| GET | `/readyz` | Public | Readiness → `{"db":"up"}` (proves DB connectivity). |
| POST | `/auth/bootstrap` | Public¹ | Create the first operator (an **admin**). Allowed only while no users exist. Body `{username, password}` (password ≥12). |
| POST | `/auth/login` | Public | Log in; sets the signed `cec_session` cookie (12 h TTL). Body `{username, password}`. Locks out after 10 fails (429). |
| POST | `/auth/logout` | Public | Clear the session cookie. |
| GET | `/auth/me` | Public² | Current session `{username, user_id, role}` or 401. |
| POST | `/auth/users` | Admin | Create an operator account. Body `{username, password}`. |
| POST | `/auth/tokens` | Admin | Mint a service-account token. Body `{label, role?}`. Returns the plaintext **once**. |
| GET | `/auth/tokens` | Admin | List tokens (metadata only — never the secret). |
| POST | `/auth/tokens/{id}/revoke` | Admin | Revoke a token immediately. |

¹ Self-disables once any user exists. ² Reads the cookie directly; returns 401 if absent/expired.

## Catalog

| Method | Path | Auth | Description |
|---|---|---|---|
| POST / GET | `/vendors` | Operator | Create / list vendors. |
| GET | `/vendors/{id}` | Operator | One vendor. |
| POST / GET | `/manufacturers` | Operator | Create / list manufacturers (warranty defaults, transferability). |
| POST / GET | `/products` | Operator | Create / list products (`serial_format_regex`, warranty months, …). |
| GET | `/products/{id}` | Operator | One product. |

## Purchases, receipts & extraction

| Method | Path | Auth | Description |
|---|---|---|---|
| POST / GET | `/purchases` | Operator | Create a purchase (+ nested `line_items`) / list. |
| GET | `/purchases/{id}` | Operator | Purchase with line items. |
| POST | `/purchases/{id}/line-items` | Operator | Append a line item. |
| POST | `/purchases/{id}/receipt` | Operator | Upload a receipt file (multipart; ≤25 MiB) → object store. |
| POST | `/purchases/{id}/allocate-costs` | Operator | Spread shipping+tax−discount across lines → per-unit landed cost. |
| POST | `/line-items/{id}/resolve` | Operator | Map a line item to a product (status → confirmed). |
| POST | `/line-items/{id}/expand` | Operator | Split a bundle line into child lines (by MSRP weight or even). |
| POST | `/purchases/from-extraction` | Operator | Pasted receipt **text** → draft purchase with unresolved lines. |
| POST | `/purchases/from-image` | Operator | Receipt **photo** (multipart; ≤25 MiB) → draft via the vision backend. |
| POST | `/purchases/from-payload` | Operator | Persist a caller-supplied §11.4 extraction payload → draft. |
| POST | `/extract-preview` | Operator | Preview extraction of pasted text (no persistence). |

## Shipments (carrier tracking)

| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/purchases/{id}/shipments` | Operator | Attach a shipment to a purchase. |
| GET | `/shipments` | Operator | List shipments (`?active=true`). |
| GET | `/shipments/{id}` | Operator | Shipment with its event history. |
| POST | `/shipments/{id}/poll` | Operator | Poll the carrier provider once. |

## Units (serialized inventory)

| Method | Path | Auth | Description |
|---|---|---|---|
| POST / GET | `/units` | Operator | Create a unit (writes `intake` event) / list. Serial is globally unique → dup = 409. |
| GET | `/units/{id}` | Operator | One unit (incl. both warranty clocks). |
| PATCH | `/units/{id}/status` | Operator | Change status (guarded transition matrix; writes `status_change`). |
| GET | `/units/{id}/events` | Operator | The unit's append-only event timeline. |
| POST | `/units/{id}/verify` | Operator | Verification pass: bind/confirm a scanned serial (regex warn-only). |
| POST | `/units/{id}/asset-tag` | Operator | Assign a `CEC-*` asset tag + return a Code128 ZPL label. |
| GET | `/units/{id}/warranty` | Operator | Two-clock warranty view + remaining days. |
| POST | `/units/{id}/recompute-warranty` | Operator | Recompute warranty + RMA readiness. |
| POST | `/units/{id}/reserve` | Operator | cec.direct: `in_stock → reserved`. |
| POST | `/units/{id}/consume` | Operator | cec.direct: `reserved/in_stock → installed`, attach to `{system_id}`. |
| POST | `/units/{id}/rma` | Operator | Open an RMA case (derives mode/proof/custody from ownership). |

## Warranty policy

| Method | Path | Auth | Description |
|---|---|---|---|
| POST / GET | `/warranty-policies` | Operator | CEC warranty policy CRUD (class × category → term months). |

## Systems (as-built machines)

| Method | Path | Auth | Description |
|---|---|---|---|
| POST / GET | `/systems` | Operator | Create / list systems (`build_id` links a cec.direct build). |
| GET | `/systems/{id}` | Operator | System with members. |
| POST | `/systems/{id}/members` | Operator | Add a member unit (invalidates the system). |
| DELETE | `/systems/{id}/members/{unit_id}` | Operator | Remove a member (invalidates). |
| POST | `/systems/{id}/validate` | Operator | Record a validation; a passing EOL/post-change restores `validated`. |
| POST | `/systems/{id}/deliver` | Operator | Ship→customer: starts the per-unit CEC warranty clock. Requires `validated`. |
| POST | `/systems/{id}/sweep` | Operator | Scan-reconcile members vs `scanned_serials`; clean sweep re-validates. |
| POST | `/systems/{id}/transfer` | Operator | Transfer a delivered system; gated on a clean sweep / validated. |
| POST | `/systems/{id}/asset-tag` | Operator | Assign a `CEC-S*` asset tag + ZPL label. |

## Stock (bulk, non-serialized)

| Method | Path | Auth | Description |
|---|---|---|---|
| POST / GET | `/stock` | Operator | Create / list bulk stock items. |
| POST | `/stock/{id}/adjust` | Operator | Signed quantity delta (guarded ≥ 0). |
| POST | `/stock/{id}/asset-tag` | Operator | Assign a `CEC-B*` asset tag + ZPL label. |

## cec.direct seam

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/availability` | Operator | In-stock serialized units per product + bulk quantities. |

(`/units/{id}/reserve` and `/consume` are listed under Units.)

## RMA lifecycle

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/rma` | Operator | List RMA cases. |
| GET / PATCH | `/rma/{id}` | Operator | Read / update a case (status/custody/tracking; `closed` is terminal). |
| POST | `/rma/{id}/proof-package` | Operator | Bundle receipt + serial + warranty terms → object store. |
| POST | `/rma/{id}/replacement` | Operator | Intake a replacement unit (predecessor retired, system re-validates). |

## No-receipt intake

| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/trade-ins` | Operator | Trade-in intake; RMA readiness from the proof situation. |
| POST | `/opening-balance` | Operator | Opening-balance intake (synthetic purchase). |

## Worklists & export

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/reorder` | Operator | Stock at/below its reorder point. |
| GET | `/receiving/reconciliation` | Operator | Delivered-but-not-received worklist. |
| GET | `/export` | Operator | Full JSON snapshot of every business table (no `app_user`). |
| GET | `/export/units.csv` | Operator | Units as CSV. |

## Operator UI (server-rendered, public read)

Browser pages render for anyone on the mesh, but their forms POST to the routes above and so
require a logged-in session: `/` (dashboard), `/ui/login`, `/ui/new`, `/ui/units[/{id}]`,
`/ui/systems[/{id}]`, `/ui/purchases[/new]`, `/ui/scan[/{unit_id}]`.

## Status codes

`200/201` ok · `400` bad request / illegal transition · `401` unauthenticated · `403` not
admin · `404` not found · `409` uniqueness conflict (e.g. duplicate serial) · `429` login
lockout · `502` extractor/carrier upstream unreachable.

> The route table is generated by hand from `crates/api/src/routes/mod.rs`,
> `crates/api/src/auth.rs`, and `crates/api/src/lib.rs`; keep it in sync when routes change.
