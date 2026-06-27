# Integrating an external app with the CEC Inventory backend

> Last updated: 2026-06-27 · Audience: an external service (e.g. the cec.direct build platform,
> a storefront, a reporting tool) that needs to read availability or drive inventory.

The backend is a plain **JSON-over-HTTP** API. There is no SDK to install — any HTTP client
works. This doc is the contract.

## 1. Network & base URL

The app is meant to run **behind the Headscale mesh**, not the public internet (cookies are not
`Secure` by default — terminate TLS / enable `Secure` if you expose it). Reach it at the mesh
address, e.g. `http://inventory.box:8080`.

- `GET /health` → `ok` (liveness, unauthenticated)
- `GET /readyz` → `{"db":"up"}` (readiness — proves DB connectivity)

## 2. Authentication — two modes

| Caller | Mechanism |
|---|---|
| A browser/operator | Cookie session: `POST /auth/login` sets a signed, HttpOnly `cec_session` cookie (12 h TTL). |
| **An external app / service** | **Bearer API token**: send `Authorization: Bearer cec_pat_…` on every request. |

**Get a token (one-time, by an admin operator):**

```sh
# An admin logs in (cookie jar), then mints a token for your app:
curl -sc jar -X POST http://inventory.box:8080/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"admin","password":"<password>"}'

curl -sb jar -X POST http://inventory.box:8080/auth/tokens \
  -H 'content-type: application/json' \
  -d '{"label":"cec.direct integration","role":"operator"}'
# → {"id":"…","label":"…","role":"operator","token":"cec_pat_XXXXXXXX","note":"shown only once"}
```

Store the `token` securely — only its hash is kept server-side; it is never shown again. Then:

```sh
curl -s http://inventory.box:8080/availability \
  -H 'authorization: Bearer cec_pat_XXXXXXXX'
```

- Tokens carry a **role** (`operator` or `admin`). `operator` reaches all data/mutation routes;
  `admin` additionally reaches `/auth/users` and `/auth/tokens`.
- Manage tokens (admin only): `GET /auth/tokens` (metadata, never the secret),
  `POST /auth/tokens/{id}/revoke`. Revocation is immediate.
- Errors: `401` (no/invalid/expired/revoked credential), `403` (authenticated but not admin),
  `409` (uniqueness conflict, e.g. duplicate serial), `429` (login lockout). Error body is
  `{"error":"…"}`.

## 3. The cec.direct seam — the canonical integration (scope §19)

A build platform consumes parts as it assembles a machine:

```
GET  /availability
  → { "serialized": [ {product_id, model, in_stock} ], "bulk": [ {product_id, quantity_on_hand} ] }

POST /units/{unit_id}/reserve            body: { "actor": "cec.direct" }
  in_stock → reserved (held for a build)

POST /units/{unit_id}/consume            body: { "system_id": "<system-uuid>", "actor": "cec.direct" }
  reserved/in_stock → installed, attached to the System
```

Link a System to your build with its `build_id`: create the System
(`POST /systems {label, build_id}`) and reference it in `consume`. Both transitions are
guarded (illegal source state → `400`), atomic (row-locked), and event-logged.

## 4. Feeding receipts / extraction (scope §3, §11)

If your app already has structured receipt data, push it straight in:

```
POST /purchases/from-payload   body: { "extraction": <§11.4 JSON>, "source_type": "...", "created_by": "..." }
```

It creates a draft purchase with **unresolved** line items for an operator to map to products.
There is also `POST /purchases/from-extraction` (pasted text → the deterministic/VLM extractor)
and `POST /purchases/from-image` (multipart photo → the vision backend).

## 5. Reading / syncing data

- Resource reads: `GET /units`, `/units/{id}`, `/systems`, `/systems/{id}`, `/purchases`,
  `/stock`, `/rma`, `/shipments`, `/units/{id}/events` (the append-only event timeline).
- Worklists: `GET /reorder`, `GET /receiving/reconciliation`.
- **Bulk export (no lock-in):** `GET /export` (full JSON snapshot of every business table) and
  `GET /export/units.csv`. Use these to mirror inventory into another system.

## 6. Conventions

- **JSON** in and out; send `content-type: application/json`.
- **Money** is sent/received as JSON **strings** (`"1599.00"`) to preserve decimal precision.
- **IDs** are UUIDs.
- **Enums** are `snake_case` strings (e.g. status `in_stock`, `with_customer`).
- Every unit mutation writes a row to the **append-only event log** (`/units/{id}/events`) — the
  audit trail for RMA/transfer disputes. Treat it as the source of truth for "what happened".
- The full route list is in `crates/api/src/routes/mod.rs`.

## 7. CSRF & cross-origin

Cookie-authenticated, state-changing requests are **same-origin checked**: the browser's
`Origin`/`Referer` host must match the server's `Host`. This is transparent to a same-origin UI.
An **external app should use a bearer token** (no cookie), which is not subject to the
same-origin check — so server-to-server calls with any/no `Origin` work normally. (There is no
CORS layer; browser cross-origin calls aren't a supported integration path.)

## 8. Not yet available (roadmap)

Per-IP rate limiting, server-side session revocation, and fine-grained token scopes (today a
token is `operator` or `admin`, not per-endpoint). Tracked in `docs/AUDIT-2026-06-27.md` /
`docs/TODO.md`.
