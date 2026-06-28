# CEC Inventory — API reference

> Last updated: 2026-06-28 · For the integration walkthrough (auth, the cec.direct seam, a
> curl tutorial), see `docs/INTEGRATION.md`. This file is the endpoint catalog; field-level
> request/response shapes for every endpoint are in **§ Endpoint schemas** at the bottom.

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
| GET | `/manufacturers/{id}` | Operator | One manufacturer. |
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
| POST | `/purchases/from-image` | Operator | Receipt **photo** (multipart; ≤25 MiB) → draft via the vision backend (blocks until done). |
| POST | `/purchases/from-image-async` | Operator | Async receipt **photo** (multipart; ≤25 MiB) → `202 {job_id}`; poll the job (warming-aware UX). |
| GET | `/purchases/from-image-jobs/{id}` | Operator | Poll an async image-extraction job → `{status, model_warm, purchase?, error?}`. |
| GET | `/extract/vlm-status` | Operator | Whether the vision model is warm (resident) vs cold-loads on first use. |
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

`200` ok · `201` created · `202` accepted (async image extraction; poll the job) · `400` bad
request / illegal status transition / **FK violation** (e.g. unknown `product_id`) · `401`
unauthenticated · `403` not admin · `404` not found · `409` uniqueness conflict (e.g. duplicate
serial) · `429` login lockout · `500` internal / unexpected DB error · `502` extractor/carrier
upstream unreachable. Error body is always `{"error":"…"}`.

> The route table + the schemas below are kept in sync with `crates/api/src/routes/mod.rs`,
> `crates/api/src/auth.rs`, and `crates/api/src/lib.rs`; update them when routes change.

---


## Endpoint schemas (request & response)


> Field-level shapes below are generated from the handler source (`crates/api/src/`) and kept
> in sync with it. `req` ✓ = required, ✗ = optional (server applies a default or accepts null).
> Money is a decimal **string** (`"1599.00"`); timestamps are ISO-8601 UTC; ids are uuid; enums
> are `snake_case`. JSON examples are illustrative (nulls show optional fields).


### Authentication & service tokens

#### `POST /auth/bootstrap`
Create the first operator (an **admin**); allowed only while no users exist. **Auth:** Public (self-disables after first use).

**Request** (`application/json`): `{ "username": "admin", "password": "<≥12 chars>" }`

**Response** `200`: `{ "ok": true, "username": "admin", "role": "admin" }`

**Errors:** 400 if already bootstrapped (create further users via `POST /auth/users`); 400 password < 12 chars.

#### `POST /auth/login`
Log in; sets the signed, HttpOnly `cec_session` cookie (12 h TTL). **Auth:** Public.

**Request** (`application/json`): `{ "username": "...", "password": "..." }`

**Response** `200`: `{ "ok": true, "username": "..." }` plus `Set-Cookie: cec_session=...`

**Errors:** 401 invalid credentials; 429 after 10 consecutive failures (15-min lockout, per user).

#### `POST /auth/logout` · `GET /auth/me`
`logout` clears the cookie → `{ "ok": true }`. `me` returns `{ "username": "...", "user_id": "<uuid>", "role": "operator|admin" }` from the cookie, or `401` if absent/expired.

#### `POST /auth/tokens` — **Admin**
Mint a service-account bearer token. The plaintext is returned **once** (only its SHA-256 hash is stored).

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| label | string | ✓ | non-empty; human label for the token |
| role | string | ✗ | `operator` (default) or `admin` |

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "label": "intranet-app",
  "role": "operator",
  "token": "cec_pat_XXXXXXXXXXXXXXXX",
  "note": "store this token now — it is shown only once and cannot be recovered"
}
```

**Errors:** 400 label empty; 401/403 if the caller is not an admin.

#### `GET /auth/tokens` · `POST /auth/tokens/{id}/revoke` · `POST /auth/users` — **Admin**
`GET /auth/tokens` lists token **metadata only** (id, label, role, created/last-used/revoked timestamps — never the secret). `revoke` → `{ "ok": true, "id": "<uuid>", "revoked": true }`, effective immediately. `POST /auth/users` creates an operator account from `{ "username": "...", "password": "<≥12 chars>" }` → `{ "ok": true, "username": "..." }`.

### Catalog — vendors · manufacturers · products

#### `POST /vendors`
Create a vendor. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| name | string | ✓ | non-empty |
| address | string | ✗ | |
| website | string | ✗ | |
| rma_url | string | ✗ | |
| rma_contact | string | ✗ | |
| account_number | string | ✗ | |
| notes | string | ✗ | |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Acme Vendor",
  "address": "123 Main St",
  "website": "https://example.com",
  "rma_url": null,
  "rma_contact": null,
  "account_number": null,
  "notes": null
}
```

**Errors:** 400 if name is empty.

#### `GET /vendors`
List all vendors, ordered by name. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "Acme Vendor",
    "address": null,
    "website": null,
    "rma_url": null,
    "rma_contact": null,
    "account_number": null,
    "notes": null
  }
]
```

**Errors:** None.

#### `GET /vendors/{id}`
Get a vendor by UUID. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Acme Vendor",
  "address": null,
  "website": null,
  "rma_url": null,
  "rma_contact": null,
  "account_number": null,
  "notes": null
}
```

**Errors:** 404 if vendor not found.

#### `POST /manufacturers`
Create a manufacturer. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| name | string | ✓ | non-empty |
| rma_url | string | ✗ | |
| rma_contact | string | ✗ | |
| warranty_policy_url | string | ✗ | |
| default_warranty_months | integer | ✗ | |
| replacement_warranty_days | integer | ✗ | |
| notes | string | ✗ | |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Intel",
  "rma_url": null,
  "rma_contact": null,
  "warranty_policy_url": null,
  "default_warranty_months": 12,
  "replacement_warranty_days": null,
  "warranty_basis_default": null,
  "warranty_transferable": null,
  "warranty_start_basis": null,
  "notes": null
}
```

**Errors:** 400 if name is empty.

#### `GET /manufacturers`
List all manufacturers, ordered by name. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "Intel",
    "rma_url": null,
    "rma_contact": null,
    "warranty_policy_url": null,
    "default_warranty_months": 12,
    "replacement_warranty_days": null,
    "warranty_basis_default": null,
    "warranty_transferable": null,
    "warranty_start_basis": null,
    "notes": null
  }
]
```

**Errors:** None.

#### `GET /manufacturers/{id}`
Get a manufacturer by UUID. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Intel",
  "rma_url": null,
  "rma_contact": null,
  "warranty_policy_url": null,
  "default_warranty_months": 12,
  "replacement_warranty_days": null,
  "warranty_basis_default": null,
  "warranty_transferable": null,
  "warranty_start_basis": null,
  "notes": null
}
```

**Errors:** 404 if manufacturer not found.

#### `POST /products`
Create a product. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| model | string | ✓ | non-empty |
| manufacturer_id | uuid | ✗ | |
| mpn | string | ✗ | |
| upc_ean | string | ✗ | |
| category | string | ✗ | |
| serialized | boolean | ✗ | defaults to true |
| default_warranty_months | integer | ✗ | |
| serial_format_regex | string | ✗ | |
| datasheet_url | string | ✗ | |
| notes | string | ✗ | |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "manufacturer_id": "550e8400-e29b-41d4-a716-446655440001",
  "model": "Core i7-9700K",
  "mpn": null,
  "upc_ean": null,
  "category": null,
  "serialized": true,
  "default_warranty_months": null,
  "serial_format_regex": null,
  "datasheet_url": null,
  "notes": null
}
```

**Errors:** 400 if model is empty.

#### `GET /products`
List all products, ordered by model. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "manufacturer_id": null,
    "model": "Core i7-9700K",
    "mpn": null,
    "upc_ean": null,
    "category": null,
    "serialized": true,
    "default_warranty_months": null,
    "serial_format_regex": null,
    "datasheet_url": null,
    "notes": null
  }
]
```

**Errors:** None.

#### `GET /products/{id}`
Get a product by UUID. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "manufacturer_id": null,
  "model": "Core i7-9700K",
  "mpn": null,
  "upc_ean": null,
  "category": null,
  "serialized": true,
  "default_warranty_months": null,
  "serial_format_regex": null,
  "datasheet_url": null,
  "notes": null
}
```

**Errors:** 404 if product not found.

### Purchases — line items · receipts · landed cost

#### `POST /purchases`
Create a new purchase with optional nested line items. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| vendor_id | uuid | ✗ | vendor entity |
| purchase_datetime | ISO-8601 UTC string | ✗ | when purchase occurred |
| order_number | string | ✗ | |
| invoice_number | string | ✗ | |
| currency | string | ✗ | default `USD` |
| subtotal | string (decimal) | ✗ | |
| tax | string (decimal) | ✗ | |
| shipping | string (decimal) | ✗ | |
| discount_total | string (decimal) | ✗ | order-level discount |
| total | string (decimal) | ✗ | |
| payment_method | string | ✗ | |
| source_type | snake_case string | ✗ | `manual` (default), `pdf`, `physical_photo`, `email`, `trade_in`, `opening_balance` |
| created_by | string | ✗ | operator identifier |
| line_items | array of objects | ✗ | each with product_id, description_as_printed, vendor_sku, quantity (default 1), unit_price, line_total, currency, is_bundle (default false) — all optional |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "vendor_id": null,
  "purchase_datetime": null,
  "order_number": null,
  "invoice_number": null,
  "currency": "USD",
  "subtotal": null,
  "tax": null,
  "shipping": null,
  "discount_total": null,
  "total": null,
  "payment_method": null,
  "source_type": "manual",
  "receipt_files": [],
  "extract_confidence": null,
  "created_by": null,
  "created_at": "2026-06-27T14:30:00Z",
  "line_items": []
}
```

**Errors:** None beyond normal DB/network.

---

#### `GET /purchases`
List all purchases, newest first. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "vendor_id": null,
    "purchase_datetime": null,
    "order_number": null,
    "invoice_number": null,
    "currency": "USD",
    "subtotal": null,
    "tax": null,
    "shipping": null,
    "discount_total": null,
    "total": null,
    "payment_method": null,
    "source_type": "manual",
    "receipt_files": [],
    "extract_confidence": null,
    "created_by": null,
    "created_at": "2026-06-27T14:30:00Z"
  }
]
```

**Errors:** None.

---

#### `GET /purchases/{id}`
Retrieve a purchase and all its line items by ID (uuid). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "vendor_id": null,
  "purchase_datetime": null,
  "order_number": null,
  "invoice_number": null,
  "currency": "USD",
  "subtotal": null,
  "tax": null,
  "shipping": null,
  "discount_total": null,
  "total": null,
  "payment_method": null,
  "source_type": "manual",
  "receipt_files": [],
  "extract_confidence": null,
  "created_by": null,
  "created_at": "2026-06-27T14:30:00Z",
  "line_items": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440001",
      "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
      "product_id": null,
      "description_as_printed": "Widget",
      "vendor_sku": "W-001",
      "quantity": 1,
      "unit_price": "100.00",
      "line_total": "100.00",
      "currency": "USD",
      "is_bundle": false,
      "parent_line_id": null,
      "allocated_landed_cost": null,
      "resolution_status": "unresolved"
    }
  ]
}
```

**Errors:** 404 purchase not found.

---

#### `POST /purchases/{id}/line-items`
Add a line item to a purchase (uuid). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| product_id | uuid | ✗ | |
| description_as_printed | string | ✗ | |
| vendor_sku | string | ✗ | |
| quantity | integer | ✗ | default `1` |
| unit_price | string (decimal) | ✗ | |
| line_total | string (decimal) | ✗ | |
| currency | string | ✗ | |
| is_bundle | boolean | ✗ | default `false` |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "product_id": null,
  "description_as_printed": "Widget",
  "vendor_sku": "W-001",
  "quantity": 1,
  "unit_price": "100.00",
  "line_total": "100.00",
  "currency": "USD",
  "is_bundle": false,
  "parent_line_id": null,
  "allocated_landed_cost": null,
  "resolution_status": "unresolved"
}
```

**Errors:** 404 purchase not found.

---

#### `POST /purchases/{id}/receipt`
Upload a receipt file for a purchase (uuid). Multipart form, 25 MiB max. **Auth:** Operator.

**Request** (multipart):

| field | type | req | notes |
|---|---|---|---|
| file | file | ✓ | first form field with filename; MIME type auto-detected |

**Response** `201`:

```json
{
  "receipt_files": [
    {
      "ref": "receipts/550e8400-e29b-41d4-a716-446655440000/550e8400_receipt.pdf",
      "filename": "receipt.pdf",
      "content_type": "application/pdf",
      "bytes": 45678,
      "uploaded_at": "2026-06-27T14:30:00Z"
    }
  ]
}
```

**Errors:** 404 purchase not found; 400 malformed multipart, no file field, or empty file.

---

#### `POST /purchases/{id}/allocate-costs`
Allocate landed costs (shipping, tax, discount) across line items for a purchase (uuid). **Auth:** Operator.

**Request** (query parameters):

| param | type | req | notes |
|---|---|---|---|
| apply_to_units | boolean | ✗ | write per-unit cost to bound inventory_unit rows; default `true` |

**Response** `200`:

```json
{
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "shipping": "50.00",
  "tax": "100.00",
  "discount_total": "0.00",
  "extra_total": "150.00",
  "lines": [
    {
      "line_id": "550e8400-e29b-41d4-a716-446655440001",
      "line_total": "1000.00",
      "allocated_extra": "150.00",
      "allocated_landed_cost": "1150.00",
      "per_unit_cost": "575.00",
      "units_updated": 2
    }
  ]
}
```

**Errors:** 404 purchase not found; 400 purchase has no line items to allocate.

---

#### `POST /line-items/{id}/resolve`
Map a line item to a canonical product (uuid) and set resolution status. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| product_id | uuid | ✓ | the canonical product |
| resolution_status | snake_case string | ✗ | `confirmed` (default), `unresolved`, `suggested` |

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "product_id": "550e8400-e29b-41d4-a716-446655440002",
  "description_as_printed": "Widget",
  "vendor_sku": "W-001",
  "quantity": 1,
  "unit_price": "100.00",
  "line_total": "100.00",
  "currency": "USD",
  "is_bundle": false,
  "parent_line_id": null,
  "allocated_landed_cost": null,
  "resolution_status": "confirmed"
}
```

**Errors:** 404 line item not found.

---

#### `POST /line-items/{id}/expand`
Expand a bundle line into child line items, one per component, with allocated costs (uuid). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| components | array of objects | ✓ | each with product_id (uuid, required), msrp (string decimal, optional) |
| allocation | snake_case string | ✗ | `msrp` (default, weight by component MSRP) or `even` (equal split) |

**Response** `201`:

```json
{
  "parent_line_id": "550e8400-e29b-41d4-a716-446655440001",
  "allocation": "msrp",
  "children": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440003",
      "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
      "product_id": "550e8400-e29b-41d4-a716-446655440002",
      "description_as_printed": null,
      "vendor_sku": null,
      "quantity": 1,
      "unit_price": "500.00",
      "line_total": "500.00",
      "currency": "USD",
      "is_bundle": false,
      "parent_line_id": "550e8400-e29b-41d4-a716-446655440001",
      "allocated_landed_cost": null,
      "resolution_status": "confirmed"
    }
  ]
}
```

**Errors:** 404 line item not found; 400 no components in array or bundle line has no line_total to allocate.

### Units & asset-tag labels

#### `POST /units`
Creates a serialized inventory unit and logs an intake event. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| product_id | uuid | ✓ | links to product catalog |
| line_item_id | uuid | ✗ | from purchase line item |
| system_id | uuid | ✗ | if allocated to a system |
| owner | enum | ✗ | `shop` (default) or `customer` |
| customer_ref | string | ✗ | customer-supplied reference |
| serial_number | string | ✗ | manufacturer serial (receipt-supplied or to-be-scanned) |
| serial_source | enum | ✗ | `receipt`, `scan`, `ocr`, `manual` |
| asset_tag | string | ✗ | internal scannable ID (typically assigned via `/units/{id}/asset-tag`) |
| condition | enum | ✗ | `new` (default), `open_box`, `used`, `refurb`, `unknown` |
| acquisition_method | enum | ✗ | `purchase` (default), `trade_in`, `rma_replacement`, `gift`, `salvage`, `opening_balance` |
| status | enum | ✗ | `in_stock` (default), `reserved`, `in_build`, `installed`, `with_customer`, `shipped`, `rma_open`, `pending_return`, `defective`, `returned`, `scrapped` |
| location_bin | string | ✗ | warehouse bin location |
| unit_cost | string | ✗ | decimal cost (money field) |
| notes | string | ✗ | intake notes |
| intake_by | string | ✗ | operator username |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "product_id": "550e8400-e29b-41d4-a716-446655440001",
  "line_item_id": null,
  "system_id": null,
  "owner": "shop",
  "customer_ref": null,
  "serial_number": "GPU-2291X",
  "serial_source": "receipt",
  "verified": false,
  "asset_tag": null,
  "condition": "new",
  "acquisition_method": "purchase",
  "status": "in_stock",
  "location_bin": "A-02-03",
  "unit_cost": "1599.00",
  "mfr_warranty_expires": "2028-06-27",
  "cec_warranty_class": null,
  "cec_warranty_start": null,
  "cec_warranty_expires": null,
  "registered": false,
  "rma_eligible": null,
  "rma_block_reason": null,
  "notes": "arrived in good condition",
  "intake_at": "2026-06-27T14:30:00Z",
  "intake_by": "alice"
}
```

**Errors:** 409 duplicate serial or asset-tag if unique constraints exist; 400 invalid product_id foreign key.

---

#### `GET /units`
Lists all inventory units in reverse intake order. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "product_id": "550e8400-e29b-41d4-a716-446655440001",
    "line_item_id": null,
    "system_id": null,
    "owner": "shop",
    "customer_ref": null,
    "serial_number": "GPU-2291X",
    "serial_source": "receipt",
    "verified": false,
    "asset_tag": null,
    "condition": "new",
    "acquisition_method": "purchase",
    "status": "in_stock",
    "location_bin": "A-02-03",
    "unit_cost": "1599.00",
    "mfr_warranty_expires": "2028-06-27",
    "cec_warranty_class": null,
    "cec_warranty_start": null,
    "cec_warranty_expires": null,
    "registered": false,
    "rma_eligible": null,
    "rma_block_reason": null,
    "notes": null,
    "intake_at": "2026-06-27T14:30:00Z",
    "intake_by": null
  }
]
```

**Errors:** None notable.

---

#### `GET /units/{id}`
Retrieves a single unit by uuid. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "product_id": "550e8400-e29b-41d4-a716-446655440001",
  "line_item_id": null,
  "system_id": null,
  "owner": "shop",
  "customer_ref": null,
  "serial_number": "GPU-2291X",
  "serial_source": "receipt",
  "verified": false,
  "asset_tag": null,
  "condition": "new",
  "acquisition_method": "purchase",
  "status": "in_stock",
  "location_bin": "A-02-03",
  "unit_cost": "1599.00",
  "mfr_warranty_expires": "2028-06-27",
  "cec_warranty_class": null,
  "cec_warranty_start": null,
  "cec_warranty_expires": null,
  "registered": false,
  "rma_eligible": null,
  "rma_block_reason": null,
  "notes": null,
  "intake_at": "2026-06-27T14:30:00Z",
  "intake_by": null
}
```

**Errors:** 404 unit not found.

---

#### `PATCH /units/{id}/status`
Changes unit status and logs the transition. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| status | enum | ✓ | target status; illegal transitions (e.g., out of terminal `scrapped`) are rejected |
| actor | string | ✗ | operator username for the event log |
| note | string | ✗ | optional reason for transition |

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "product_id": "550e8400-e29b-41d4-a716-446655440001",
  "line_item_id": null,
  "system_id": null,
  "owner": "shop",
  "customer_ref": null,
  "serial_number": "GPU-2291X",
  "serial_source": "receipt",
  "verified": true,
  "asset_tag": "CEC-U-A1B2C3D4",
  "condition": "new",
  "acquisition_method": "purchase",
  "status": "reserved",
  "location_bin": "A-02-03",
  "unit_cost": "1599.00",
  "mfr_warranty_expires": "2028-06-27",
  "cec_warranty_class": null,
  "cec_warranty_start": null,
  "cec_warranty_expires": null,
  "registered": false,
  "rma_eligible": null,
  "rma_block_reason": null,
  "notes": null,
  "intake_at": "2026-06-27T14:30:00Z",
  "intake_by": null
}
```

**Errors:** 400 illegal status transition (e.g., `in_stock -> scrapped` may not go to `reserved`); 404 unit not found.

---

#### `GET /units/{id}/events`
Lists all event log entries for a unit by uuid, ordered by timestamp. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "660e8400-e29b-41d4-a716-446655440000",
    "unit_id": "550e8400-e29b-41d4-a716-446655440000",
    "event_type": "intake",
    "from_value": null,
    "to_value": "in_stock",
    "actor": "alice",
    "at": "2026-06-27T14:30:00Z",
    "system_id": null,
    "rma_case_id": null,
    "detail": {
      "serial_number": "GPU-2291X",
      "asset_tag": null,
      "condition": "new",
      "acquisition_method": "purchase"
    }
  },
  {
    "id": "660e8400-e29b-41d4-a716-446655440001",
    "unit_id": "550e8400-e29b-41d4-a716-446655440000",
    "event_type": "verify",
    "from_value": null,
    "to_value": "GPU-2291X",
    "actor": "bob",
    "at": "2026-06-27T15:00:00Z",
    "system_id": null,
    "rma_case_id": null,
    "detail": {
      "bound_from_scan": false,
      "format_valid": true
    }
  }
]
```

**Errors:** 404 unit not found.

---

#### `POST /units/{id}/verify`
Verifies or binds a unit's serial number by scan and validates against the product's serial format regex. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| scanned_serial | string | ✓ | the scanned serial to bind or verify against |
| actor | string | ✗ | operator username for the event log |

**Response** `200`:

```json
{
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "verified": true,
  "matched": true,
  "bound_from_scan": false,
  "format_valid": true,
  "warnings": []
}
```

**Errors:** 404 unit not found.

---

#### `POST /units/{id}/asset-tag`
Assigns an internal scannable asset tag (or reprints if already assigned) and returns the ZPL thermal label. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "asset_tag": "CEC-U-A1B2C3D4",
  "kind": "unit",
  "zpl": "^XA^FO40,40^BCN,120,Y,N,N^FDCEC-U-A1B2C3D4^FS^FO40,180^A0N,28,28^FDCEC-U-A1B2C3D4^FS^XZ",
  "label_text": "CEC-U-A1B2C3D4"
}
```

**Errors:** 404 unit not found.

---

#### `POST /systems/{id}/asset-tag`
Assigns an internal scannable asset tag to a system (or reprints if already assigned) and returns the ZPL thermal label. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "asset_tag": "CEC-S-B2C3D4E5",
  "kind": "system",
  "zpl": "^XA^FO40,40^BCN,120,Y,N,N^FDCEC-S-B2C3D4E5^FS^FO40,180^A0N,28,28^FDCEC-S-B2C3D4E5^FS^XZ",
  "label_text": "CEC-S-B2C3D4E5"
}
```

**Errors:** 404 system not found.

---

#### `POST /stock/{id}/asset-tag`
Assigns an internal scannable asset tag to a stock item (or reprints if already assigned) and returns the ZPL thermal label. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "asset_tag": "CEC-B-C3D4E5F6",
  "kind": "stock item",
  "zpl": "^XA^FO40,40^BCN,120,Y,N,N^FDCEC-B-C3D4E5F6^FS^FO40,180^A0N,28,28^FDCEC-B-C3D4E5F6^FS^XZ",
  "label_text": "CEC-B-C3D4E5F6"
}
```

**Errors:** 404 stock item not found.

### Systems (as-built machines)

#### `POST /systems`
Create a new system (the as-built/as-delivered machine). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| label | string | ✗ | optional system label |
| asset_tag | string | ✗ | optional asset tag |
| build_id | uuid | ✗ | optional associated build id |
| cec_warranty_class | string | ✗ | `full`, `refurb`, or `none` (default: none) |
| notes | string | ✗ | optional notes |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "label": "System A",
  "asset_tag": "ASSET-001",
  "build_id": "550e8400-e29b-41d4-a716-446655440001",
  "current_owner": "shop",
  "customer_ref": null,
  "status": "in_build",
  "delivery_datetime": null,
  "cec_warranty_class": null,
  "validation_state": "invalidated",
  "provenance_stale": false,
  "last_validated_at": null,
  "last_validated_by": null,
  "notes": null
}
```

**Errors:** none specific.

---

#### `GET /systems`
List all systems. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "label": "System A",
    "asset_tag": "ASSET-001",
    "build_id": "550e8400-e29b-41d4-a716-446655440001",
    "current_owner": "shop",
    "customer_ref": null,
    "status": "in_build",
    "delivery_datetime": null,
    "cec_warranty_class": null,
    "validation_state": "invalidated",
    "provenance_stale": false,
    "last_validated_at": null,
    "last_validated_by": null,
    "notes": null
  }
]
```

**Errors:** none specific.

---

#### `GET /systems/{id}`
Get a system with its member units (uuid path param). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "label": "System A",
  "asset_tag": "ASSET-001",
  "build_id": "550e8400-e29b-41d4-a716-446655440001",
  "current_owner": "shop",
  "customer_ref": null,
  "status": "in_build",
  "delivery_datetime": null,
  "cec_warranty_class": null,
  "validation_state": "invalidated",
  "provenance_stale": false,
  "last_validated_at": null,
  "last_validated_by": null,
  "notes": null,
  "members": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440100",
      "serial_number": "SN123456",
      "owner": "shop",
      "status": "in_build",
      "cec_warranty_class": null,
      "cec_warranty_expires": null
    }
  ]
}
```

**Errors:** 404 system not found.

---

#### `POST /systems/{id}/members`
Add a unit to a system (invalidates the system per scope §6.4). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| unit_id | uuid | ✓ | unit to add to the system |
| actor | string | ✗ | optional actor/performer label |

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "label": "System A",
  "asset_tag": "ASSET-001",
  "build_id": "550e8400-e29b-41d4-a716-446655440001",
  "current_owner": "shop",
  "customer_ref": null,
  "status": "in_build",
  "delivery_datetime": null,
  "cec_warranty_class": null,
  "validation_state": "invalidated",
  "provenance_stale": false,
  "last_validated_at": null,
  "last_validated_by": null,
  "notes": null,
  "members": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440100",
      "serial_number": "SN123456",
      "owner": "shop",
      "status": "in_build",
      "cec_warranty_class": null,
      "cec_warranty_expires": null
    }
  ]
}
```

**Errors:** 404 unit not found.

---

#### `DELETE /systems/{id}/members/{unit_id}`
Remove a unit from a system (invalidates the system per scope §6.4). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "label": "System A",
  "asset_tag": "ASSET-001",
  "build_id": "550e8400-e29b-41d4-a716-446655440001",
  "current_owner": "shop",
  "customer_ref": null,
  "status": "in_build",
  "delivery_datetime": null,
  "cec_warranty_class": null,
  "validation_state": "invalidated",
  "provenance_stale": false,
  "last_validated_at": null,
  "last_validated_by": null,
  "notes": null,
  "members": []
}
```

**Errors:** 404 unit is not a member of this system.

---

#### `POST /systems/{id}/validate`
Record a system validation (passing EOL/post-change/periodic restores validated state; fail sets invalidated). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| validation_type | string | ✓ | `eol`, `post_change`, `periodic`, `pre_transfer`, or `sweep` |
| trigger | string | ✗ | `build_complete`, `rma`, `parts_swap`, `service`, `transfer_request`, or `audit` |
| result | string | ✓ | `pass` or `fail` |
| performed_by | string | ✗ | optional performer label |
| evidence_refs | array | ✗ | optional array of evidence references (default: []) |
| notes | string | ✗ | optional notes |

**Response** `200`:

```json
{
  "validation_id": "550e8400-e29b-41d4-a716-446655440050",
  "validation_state": "validated"
}
```

**Errors:** none specific.

---

#### `POST /systems/{id}/deliver`
Deliver a system to a customer (flips ownership to customer, stamps delivery time, starts CEC warranty per member unit). Requires system to be validated. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| customer_ref | string | ✓ | customer reference/identifier |
| cec_warranty_class | string | ✗ | `full`, `refurb`, or `none` (default: `full`) |
| performed_by | string | ✗ | optional performer label |

**Response** `200`:

```json
{
  "system_id": "550e8400-e29b-41d4-a716-446655440000",
  "delivery_datetime": "2026-06-28T15:30:45.123456Z",
  "units_delivered": 2
}
```

**Errors:** 400 system must be validated before delivery; 400 system already delivered.

---

#### `POST /systems/{id}/sweep`
Scan and reconcile the system's members against the scanned serial set (per scope §6.5). Clean sweep re-validates the system and authorizes a transfer; discrepancies record fail. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| scanned_serials | array(string) | ✓ | list of scanned serial numbers |
| performed_by | string | ✗ | optional performer label |

**Response** `200`:

```json
{
  "validation_id": "550e8400-e29b-41d4-a716-446655440051",
  "overall": "clean",
  "reconciliation": {
    "per_unit": [
      {
        "unit_id": "550e8400-e29b-41d4-a716-446655440100",
        "serial": "SN123456",
        "result": "matched"
      }
    ],
    "unexpected_extra": [],
    "overall": "clean"
  },
  "validation_state": "validated"
}
```

**Errors:** none specific.

---

#### `POST /systems/{id}/transfer`
Transfer a delivered system to a new owner (per scope §6.5). Precondition: a clean sweep or currently-validated system. Manufacturer warranty carries per-part only where the maker allows it. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| to_owner_ref | string | ✓ | new owner reference/identifier |
| performed_by | string | ✗ | optional performer label |
| sweep_id | uuid | ✗ | optional authorizing clean sweep validation id |
| cec_warranty_outcome | string | ✗ | `carried`, `reset`, `prorated`, or `declined` (default: `carried`) |
| cec_transfer_fee | string | ✗ | optional decimal fee (string format) |

**Response** `200`:

```json
{
  "transfer_id": "550e8400-e29b-41d4-a716-446655440060",
  "result": "completed",
  "from_owner_ref": "CUST-001",
  "to_owner_ref": "CUST-002",
  "mfr_warranty_outcome": [
    {
      "unit_id": "550e8400-e29b-41d4-a716-446655440100",
      "outcome": "carried"
    }
  ]
}
```

**Errors:** 400 only a delivered (customer-owned) system can be transferred; 400 transfer blocked: a clean parts sweep is required (scope §6.5).

### Warranty & RMA lifecycle

#### `GET /units/{id}/warranty`
Read the stored warranty state for a unit (id is uuid). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "mfr_warranty_expires": "2026-12-31",
  "mfr_days_left": 180,
  "cec_warranty_class": "full",
  "cec_warranty_expires": "2027-12-31",
  "cec_days_left": 550,
  "cec_warranty_active": true,
  "rma_eligible": true,
  "rma_block_reason": null
}
```

**Errors:** 404 unit not found.

---

#### `POST /units/{id}/recompute-warranty`
Recompute and persist both warranty clocks and RMA readiness for a unit (id is uuid). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "mfr_warranty_expires": "2026-12-31",
  "mfr_days_left": 180,
  "cec_warranty_class": "full",
  "cec_warranty_expires": "2027-12-31",
  "cec_days_left": 550,
  "cec_warranty_active": true,
  "rma_eligible": true,
  "rma_block_reason": null
}
```

**Errors:** 404 unit not found.

---

#### `POST /warranty-policies`
Create a CEC warranty policy. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| warranty_class | string | ✓ | enum: `full`, `refurb`, `none` |
| category | string | ✗ | optional category scope |
| term_months | integer | ✓ | duration of coverage |
| transferable | boolean | ✗ | default false |
| reset_on_transfer | boolean | ✗ | default false |
| clock_pauses_when_invalidated | boolean | ✗ | default false |
| notes | string | ✗ | policy notes |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "warranty_class": "full",
  "category": "laptop",
  "term_months": 12,
  "transferable": true,
  "reset_on_transfer": false,
  "clock_pauses_when_invalidated": false,
  "notes": "Standard extended warranty"
}
```

---

#### `GET /warranty-policies`
List all CEC warranty policies. **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "warranty_class": "full",
    "category": null,
    "term_months": 12,
    "transferable": false,
    "reset_on_transfer": false,
    "clock_pauses_when_invalidated": false,
    "notes": null
  }
]
```

---

#### `POST /units/{id}/rma`
Open an RMA case on a failed unit (id is uuid). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| party | string | ✗ | enum: `vendor`, `manufacturer` |
| execution_mode | string | ✗ | enum: `cec_managed`, `customer_ships_to_cec`, `customer_managed_assist`; defaults to unit owner |
| fault_description | string | ✗ | description of the failure |
| advance_replacement | boolean | ✗ | default false |
| return_due_date | string | ✗ | ISO-8601 date |
| rma_number | string | ✗ | external RMA case identifier |
| actor | string | ✗ | user/operator identifier |
| notes | string | ✗ | case notes |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "owner_at_failure": "shop",
  "party": "vendor",
  "execution_mode": "cec_managed",
  "proof_source": "cec_receipt",
  "custody": "at_cec",
  "rma_number": "RMA-001",
  "fault_description": "Unit powers on intermittently",
  "status": "open",
  "assist_artifacts": null,
  "advance_replacement": false,
  "auth_hold_ref": null,
  "return_due_date": "2026-07-28",
  "opened_at": "2026-06-28T10:00:00Z",
  "closed_at": null,
  "shipped_at": null,
  "return_tracking": null,
  "replacement_unit_id": null,
  "resolution": null,
  "notes": null
}
```

**Errors:** 404 unit not found.

---

#### `GET /rma`
List all RMA cases (ordered by opened_at descending). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440001",
    "unit_id": "550e8400-e29b-41d4-a716-446655440000",
    "owner_at_failure": "shop",
    "party": "vendor",
    "execution_mode": "cec_managed",
    "proof_source": "cec_receipt",
    "custody": "at_cec",
    "rma_number": "RMA-001",
    "fault_description": "Unit powers on intermittently",
    "status": "open",
    "assist_artifacts": null,
    "advance_replacement": false,
    "auth_hold_ref": null,
    "return_due_date": "2026-07-28",
    "opened_at": "2026-06-28T10:00:00Z",
    "closed_at": null,
    "shipped_at": null,
    "return_tracking": null,
    "replacement_unit_id": null,
    "resolution": null,
    "notes": null
  }
]
```

---

#### `GET /rma/{id}`
Fetch a single RMA case (id is uuid). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "owner_at_failure": "shop",
  "party": "vendor",
  "execution_mode": "cec_managed",
  "proof_source": "cec_receipt",
  "custody": "at_cec",
  "rma_number": "RMA-001",
  "fault_description": "Unit powers on intermittently",
  "status": "open",
  "assist_artifacts": null,
  "advance_replacement": false,
  "auth_hold_ref": null,
  "return_due_date": "2026-07-28",
  "opened_at": "2026-06-28T10:00:00Z",
  "closed_at": null,
  "shipped_at": null,
  "return_tracking": null,
  "replacement_unit_id": null,
  "resolution": null,
  "notes": null
}
```

**Errors:** 404 rma case not found.

---

#### `PATCH /rma/{id}`
Update a case's status, custody, tracking, and metadata (id is uuid). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| status | string | ✗ | enum: `open`, `info_provided_to_customer`, `awaiting_customer_action`, `shipped_to_vendor`, `awaiting_replacement`, `replacement_received`, `replacement_with_customer`, `refunded`, `denied`, `closed`; terminal if `closed` |
| custody | string | ✗ | enum: `at_cec`, `with_customer`, `in_transit_to_cec`, `in_transit_to_vendor`, `in_transit_to_customer` |
| rma_number | string | ✗ | external case identifier |
| return_tracking | string | ✗ | carrier tracking number |
| resolution | string | ✗ | resolution notes |
| auth_hold_ref | string | ✗ | authorization/hold reference |
| actor | string | ✗ | user/operator identifier |

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "owner_at_failure": "shop",
  "party": "vendor",
  "execution_mode": "cec_managed",
  "proof_source": "cec_receipt",
  "custody": "in_transit_to_vendor",
  "rma_number": "RMA-001",
  "fault_description": "Unit powers on intermittently",
  "status": "shipped_to_vendor",
  "assist_artifacts": null,
  "advance_replacement": false,
  "auth_hold_ref": null,
  "return_due_date": "2026-07-28",
  "opened_at": "2026-06-28T10:00:00Z",
  "closed_at": null,
  "shipped_at": "2026-06-29T12:00:00Z",
  "return_tracking": "1Z999AA10123456784",
  "replacement_unit_id": null,
  "resolution": null,
  "notes": null
}
```

**Errors:** 400 if case is closed and status change requested; 404 rma case not found.

---

#### `POST /rma/{id}/proof-package`
Build the proof-of-purchase package for RMA filing (id is uuid). **Auth:** Operator.

**Request** (`application/json`):

_No request body._

**Response** `200`:

```json
{
  "rma_case_id": "550e8400-e29b-41d4-a716-446655440001",
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "serial_number": "SN-123456",
  "asset_tag": "AT-789",
  "product": {
    "model": "ThinkPad X1 Carbon",
    "mpn": "20QH003CUS",
    "manufacturer": "Lenovo"
  },
  "mfr_warranty_expires": "2026-12-31",
  "purchase": {
    "datetime": "2024-01-15T00:00:00Z",
    "vendor": "TechVendor Inc.",
    "receipt_files": ["receipt_001.pdf"]
  },
  "generated_at": "2026-06-28T10:00:00Z"
}
```

**Errors:** 404 rma case not found, 404 unit not found.

---

#### `POST /rma/{id}/replacement`
Intake a replacement unit for an RMA case (id is uuid). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| serial_number | string | ✗ | serial number of replacement unit |
| refurbished | boolean | ✗ | default false; if true, condition is `refurb` and cec_warranty_class is `refurb`; else `new` and `full` |
| unit_cost | string | ✗ | cost of the replacement (JSON decimal string) |
| actor | string | ✗ | user/operator identifier |

**Response** `201`:

```json
{
  "replacement_unit_id": "550e8400-e29b-41d4-a716-446655440002",
  "replaces_unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "condition": "new",
  "system_revalidation_required": true
}
```

**Errors:** 400 failed unit has no product; 404 rma case not found, 404 unit not found.

### cec.direct seam · no-receipt intake · bulk stock

#### `GET /availability`
Read in-stock serialized units per product and bulk stock quantity-on-hand. **Auth:** Operator.

**Request:**

_No request body._

**Response** `200`:

```json
{
  "serialized": [
    {
      "product_id": "550e8400-e29b-41d4-a716-446655440000",
      "model": "XPS 13",
      "in_stock": 5
    }
  ],
  "bulk": [
    {
      "product_id": "550e8400-e29b-41d4-a716-446655440000",
      "quantity_on_hand": 120
    }
  ]
}
```

**Errors:** None.

---

#### `POST /units/{id}/reserve`
Transition a unit from in-stock to reserved, uuid {id}. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| actor | string | ✗ | operator/system name logging the action |

**Response** `200`:

```json
{
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "reserved"
}
```

**Errors:** 404 unit not found; 400 unit not in "in_stock" status.

---

#### `POST /units/{id}/consume`
Transition a unit to installed and attach to a system (build), uuid {id}. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| system_id | uuid | ✓ | target system (build) to attach unit to |
| actor | string | ✗ | operator/system name logging the action |

**Response** `200`:

```json
{
  "unit_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "installed",
  "system_id": "660e8400-e29b-41d4-a716-446655440000"
}
```

**Errors:** 404 unit not found or system not found; 400 unit not in "in_stock" or "reserved" status.

---

#### `POST /trade-ins`
Create a trade-in intake record with units. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| customer_ref | string | ✗ | external customer reference |
| source_notes | string | ✗ | notes about the trade-in source |
| proof_of_purchase_status | enum | ✓ | one of: `provided`, `customer_has_will_send`, `customer_lacks`, `none` |
| proof_files | array | ✗ | proof document metadata (defaults to []) |
| units | object[] | ✓ | each: product_id (uuid, req), serial_number (str), serial_source (enum), condition (enum, default `unknown`), location_bin (str), unit_cost (decimal str), notes (str), intake_by (str) — all optional except product_id |

**Response** `201`:

```json
{
  "trade_in_id": "550e8400-e29b-41d4-a716-446655440000",
  "purchase_id": null,
  "unit_ids": [
    "660e8400-e29b-41d4-a716-446655440001",
    "660e8400-e29b-41d4-a716-446655440002"
  ]
}
```

**Errors:** None specific.

---

#### `POST /opening-balance`
Create an opening-balance intake record (synthetic purchase) with units. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| vendor_id | uuid | ✗ | vendor for the synthetic purchase |
| purchase_datetime | ISO-8601 string | ✗ | purchase date if origin known |
| origin_known | boolean | ✗ | whether origin (vendor/date/cost) is reconstructed; defaults to false; unknown origin blocks RMA |
| units | object[] | ✓ | each: product_id (uuid, req), serial_number (str), serial_source (enum), condition (enum, default `unknown`), location_bin (str), unit_cost (decimal str), notes (str), intake_by (str) — all optional except product_id |

**Response** `201`:

```json
{
  "trade_in_id": null,
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "unit_ids": [
    "660e8400-e29b-41d4-a716-446655440001"
  ]
}
```

**Errors:** None specific.

---

#### `POST /stock`
Create a bulk stock item for a product. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| product_id | uuid | ✓ | product for this stock item |
| location_bin | string | ✗ | warehouse bin/location |
| asset_tag | string | ✗ | physical asset tag |
| quantity_on_hand | i32 | ✗ | initial quantity; defaults to 0, must be ≥ 0 |
| reorder_point | i32 | ✗ | low-stock threshold |
| notes | string | ✗ | notes |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "product_id": "660e8400-e29b-41d4-a716-446655440000",
  "location_bin": "A-12",
  "asset_tag": "ST-001",
  "quantity_on_hand": 50,
  "reorder_point": 10,
  "notes": "bulk resistors"
}
```

**Errors:** 400 quantity_on_hand negative.

---

#### `GET /stock`
List all bulk stock items. **Auth:** Operator.

**Request:**

_No request body._

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "product_id": "660e8400-e29b-41d4-a716-446655440000",
    "location_bin": "A-12",
    "asset_tag": "ST-001",
    "quantity_on_hand": 50,
    "reorder_point": 10,
    "notes": "bulk resistors"
  }
]
```

**Errors:** None.

---

#### `POST /stock/{id}/adjust`
Adjust a stock item's quantity by signed delta, uuid {id}. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| delta | i32 | ✓ | signed change; e.g., +50 for receipt, -3 for consumed |
| note | string | ✗ | reason/notes for the adjustment |

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "product_id": "660e8400-e29b-41d4-a716-446655440000",
  "location_bin": "A-12",
  "asset_tag": "ST-001",
  "quantity_on_hand": 55,
  "reorder_point": 10,
  "notes": "bulk resistors"
}
```

**Errors:** 404 stock item not found; 400 adjustment would make quantity negative or overflow.

### Shipments · worklists · export

#### `POST /purchases/{id}/shipments`
Create a shipment for a purchase (uuid {id}). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| carrier | string | ✗ | one of: `usps`, `ups`, `fedex`, `dhl`, `other` |
| tracking_number | string | ✗ | carrier tracking identifier |
| tracking_url | string | ✗ | carrier tracking URL |
| expected_delivery_date | string | ✗ | ISO-8601 date (YYYY-MM-DD) |
| line_item_ids | array[string] | ✗ | list of purchase line item uuids |
| notes | string | ✗ | operator notes |

**Response** `201`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "carrier": "ups",
  "tracking_number": "1Z999AA10123456784",
  "tracking_url": "https://example.com/track",
  "status": "pre_transit",
  "expected_delivery_date": "2026-07-01",
  "shipped_at": "2026-06-28T12:34:56Z",
  "delivered_at": null,
  "last_polled_at": null,
  "poll_state": "active",
  "line_item_ids": ["550e8400-e29b-41d4-a716-446655440000"],
  "notes": "Rush shipment"
}
```

**Errors:** 404 if purchase not found.

#### `GET /shipments`
List all shipments, optionally filtered to active polls. **Auth:** Operator.

**Query** params:

| param | type | notes |
|---|---|---|
| active | boolean | if `true`, return only shipments with `poll_state = "active"` |

**Response** `200`:

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
    "carrier": "ups",
    "tracking_number": "1Z999AA10123456784",
    "tracking_url": "https://example.com/track",
    "status": "in_transit",
    "expected_delivery_date": "2026-07-01",
    "shipped_at": "2026-06-28T12:34:56Z",
    "delivered_at": null,
    "last_polled_at": "2026-06-28T13:00:00Z",
    "poll_state": "active",
    "line_item_ids": ["550e8400-e29b-41d4-a716-446655440000"],
    "notes": null
  }
]
```

#### `GET /shipments/{id}`
Get a shipment by uuid with its full event history. **Auth:** Operator.

**Response** `200`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "carrier": "ups",
  "tracking_number": "1Z999AA10123456784",
  "tracking_url": "https://example.com/track",
  "status": "delivered",
  "expected_delivery_date": "2026-07-01",
  "shipped_at": "2026-06-28T12:34:56Z",
  "delivered_at": "2026-06-29T14:15:00Z",
  "last_polled_at": "2026-06-29T14:15:00Z",
  "poll_state": "stopped",
  "line_item_ids": ["550e8400-e29b-41d4-a716-446655440000"],
  "notes": null,
  "events": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "shipment_id": "550e8400-e29b-41d4-a716-446655440000",
      "event_status": "label_created",
      "carrier_description": "Label created",
      "location": null,
      "occurred_at": "2026-06-28T12:34:56Z",
      "polled_at": "2026-06-28T13:00:00Z",
      "raw": {"tracking_id": "1Z999AA10123456784"}
    }
  ]
}
```

**Errors:** 404 if shipment not found.

#### `POST /shipments/{id}/poll`
Run one poll tick against the configured carrier provider for shipment uuid {id}. **Auth:** Operator.

**Request**: _No request body._

**Response** `200`:

```json
{
  "provider": "usps",
  "outcome": {
    "shipment_id": "550e8400-e29b-41d4-a716-446655440000",
    "new_events": 1,
    "status": "in_transit",
    "poll_state": "active"
  },
  "shipment": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
    "carrier": "usps",
    "tracking_number": "9400111899223456789012",
    "tracking_url": "https://tools.usps.com/go/TrackConfirmAction?tLabels=9400111899223456789012",
    "status": "in_transit",
    "expected_delivery_date": "2026-07-01",
    "shipped_at": "2026-06-28T12:34:56Z",
    "delivered_at": null,
    "last_polled_at": "2026-06-28T14:30:00Z",
    "poll_state": "active",
    "line_item_ids": null,
    "notes": null
  }
}
```

**Errors:** 404 if shipment not found; 502 if carrier provider unreachable.

#### `GET /reorder`
Stock at or below reorder point (cross-cutting reorder worklist). **Auth:** Operator.

**Response** `200`:

```json
[
  {
    "stock_id": "550e8400-e29b-41d4-a716-446655440000",
    "product_id": "550e8400-e29b-41d4-a716-446655440000",
    "model": "ThinkPad X1 Carbon",
    "location_bin": "A-02-01",
    "quantity_on_hand": 2,
    "reorder_point": 5
  }
]
```

#### `GET /receiving/reconciliation`
Shipments marked delivered by carrier but with no intake units yet (receiving "to receive" worklist). **Auth:** Operator.

**Response** `200`:

```json
{
  "delivered_not_received": [
    {
      "shipment_id": "550e8400-e29b-41d4-a716-446655440000",
      "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
      "tracking_number": "1Z999AA10123456784"
    }
  ],
  "count": 1
}
```

#### `GET /export`
Full inventory snapshot as JSON (portable, no lock-in). Excludes `app_user` (contains password hashes). **Auth:** Operator.

**Response** `200`:

```json
{
  "exported_at": "2026-06-28T12:34:56Z",
  "vendors": [{"id": "550e8400-e29b-41d4-a716-446655440000", "name": "CDW"}],
  "vendor_return_policies": [],
  "manufacturers": [{"id": "550e8400-e29b-41d4-a716-446655440000", "name": "Lenovo"}],
  "products": [{"id": "550e8400-e29b-41d4-a716-446655440000", "model": "ThinkPad X1"}],
  "purchases": [],
  "line_items": [],
  "shipments": [],
  "shipment_events": [],
  "units": [],
  "stock": [],
  "systems": [],
  "system_validations": [],
  "system_transfers": [],
  "cec_warranty_policies": [],
  "rma_cases": [],
  "trade_ins": [],
  "trade_in_units": [],
  "unit_events": []
}
```

#### `GET /export/units.csv`
All units as CSV for portability. **Auth:** Operator. Returns `text/csv` (not JSON).

**Response** `200` (text/csv):

```
id,serial_number,product_id,status,owner,condition,asset_tag,location_bin,unit_cost
550e8400-e29b-41d4-a716-446655440000,SN123456,550e8400-e29b-41d4-a716-446655440000,in_stock,shop,refurb,TAG-001,A-01-01,599.99
```

### Receipt extraction & vision

#### `POST /extract-preview`
Preview extraction of pasted receipt text (no persistence). **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| text | string | ✓ | Receipt text to extract |
| vendor_hint | string | ✗ | Optional vendor name hint |

**Response** `200`:

```json
{
  "vendor": "Acme Corp",
  "purchase_datetime": "2026-01-15T14:30:00",
  "order_number": "ORD-12345",
  "invoice_number": "INV-67890",
  "currency": "USD",
  "line_items": [
    {
      "description": "Widget Pro",
      "vendor_sku": "WDG-001",
      "quantity": 2,
      "unit_price": "99.99",
      "line_total": "199.98",
      "serials": ["SN123456"],
      "is_bundle": false,
      "confidence": 0.92
    }
  ],
  "shipments": [],
  "subtotal": "199.98",
  "tax": "15.99",
  "shipping": "10.00",
  "discount_total": "0.00",
  "total": "225.97",
  "field_confidence": { "vendor": 0.95, "total": 0.98, "datetime": 0.85 }
}
```

**Errors:** 502 if the extractor service is unreachable or unresponsive.

---

#### `POST /purchases/from-extraction`
Extract pasted receipt text and persist as a draft purchase with unresolved line items. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| text | string | ✓ | Receipt text to extract |
| vendor_hint | string | ✗ | Optional vendor name hint |
| vendor_id | uuid | ✗ | Link to a known vendor |
| created_by | string | ✗ | Operator name (capped 200 chars) |
| source_type | string | ✗ | enum: `manual` (default), `physical_photo`, `pdf`, `email`, `trade_in`, `opening_balance` |

**Response** `201`:

```json
{
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "engine": "template",
  "vendor": "Acme Corp",
  "line_item_ids": ["660e8400-e29b-41d4-a716-446655440000"],
  "line_item_count": 1,
  "needs_resolution": true
}
```

**Errors:** 400 if extraction payload exceeds 256 KiB; 400 if >1000 line items; 400 if line quantity invalid (not 1–1,000,000) or price negative; 502 if extractor unreachable.

---

#### `POST /purchases/from-image`
Extract receipt image via vision backend and persist as a draft purchase. **Auth:** Operator.

**Request** (`multipart/form-data`, 25 MiB limit):

| field | type | req | notes |
|---|---|---|---|
| (file) | image file | ✓ | First file field; JPEG/PNG/WebP/GIF; unknown media types default to image/jpeg |
| vendor_hint | text | ✗ | Optional vendor name hint (capped 200 chars) |
| created_by | text | ✗ | Operator name (capped 200 chars) |

**Response** `201`:

```json
{
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "engine": "vision",
  "vendor": "Acme Corp",
  "line_item_ids": ["660e8400-e29b-41d4-a716-446655440000"],
  "line_item_count": 1,
  "needs_resolution": true
}
```

**Errors:** 400 if multipart is malformed, no image file present, or image is empty; 400 if extraction payload exceeds 256 KiB; 400 if >1000 line items or invalid line data; 502 if vision backend unreachable.

---

#### `POST /purchases/from-image-async`
Non-blocking receipt image extraction. Registers an in-memory job and returns immediately; UI polls job status while the model warms and extraction runs. **Auth:** Operator.

**Request** (`multipart/form-data`, 25 MiB limit):

| field | type | req | notes |
|---|---|---|---|
| (file) | image file | ✓ | First file field; JPEG/PNG/WebP/GIF; unknown media types default to image/jpeg |
| vendor_hint | text | ✗ | Optional vendor name hint (capped 200 chars) |
| created_by | text | ✗ | Operator name (capped 200 chars) |

**Response** `202`:

```json
{
  "job_id": "770e8400-e29b-41d4-a716-446655440000",
  "status": "processing"
}
```

**Errors:** 400 if multipart is malformed or image invalid.

---

#### `GET /purchases/from-image-jobs/{id}`
Poll an async extraction job (id is uuid). Returns the job's status, model warm state, result if ready, and error if failed. Jobs expire after 30 minutes or on API restart. **Auth:** Operator.

**Response** `200`:

```json
{
  "status": "ready",
  "model_warm": true,
  "purchase": {
    "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
    "engine": "vision",
    "vendor": "Acme Corp",
    "line_item_ids": ["660e8400-e29b-41d4-a716-446655440000"],
    "line_item_count": 1,
    "needs_resolution": true
  },
  "created_at": 1234567890
}
```

**Errors:** 404 if job not found or expired.

---

#### `GET /extract/vlm-status`
Query whether the vision model is warm (resident). Best-effort; always returns 200 with a status object. **Auth:** Operator.

**Response** `200`:

```json
{
  "warm": true,
  "detail": "model loaded"
}
```

(On extractor failure, returns `{"warm": false, "detail": "…"}` with a description of the error.)

---

#### `POST /purchases/from-payload`
Persist a caller-supplied §11.4 extraction payload (from an external vision service, operator pass, or prior export) as a draft purchase. **Auth:** Operator.

**Request** (`application/json`):

| field | type | req | notes |
|---|---|---|---|
| extraction | object | ✓ | §11.4 extraction object (vendor, purchase_datetime, line_items[], etc.) |
| vendor_id | uuid | ✗ | Link to a known vendor |
| created_by | string | ✗ | Operator name (capped 200 chars) |
| source_type | string | ✗ | enum: `manual` (default), `physical_photo`, `pdf`, `email`, `trade_in`, `opening_balance` |

**Response** `201`:

```json
{
  "purchase_id": "550e8400-e29b-41d4-a716-446655440000",
  "engine": null,
  "vendor": "Acme Corp",
  "line_item_ids": ["660e8400-e29b-41d4-a716-446655440000"],
  "line_item_count": 1,
  "needs_resolution": true
}
```

**Errors:** 400 if extraction is not a JSON object; 400 if payload exceeds 256 KiB; 400 if >1000 line items or invalid line data.
