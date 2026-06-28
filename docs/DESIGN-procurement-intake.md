# DESIGN — Automated procurement intake (email ingest + order/shipment tracking)

> Last updated: 2026-06-28 · Status: **design for review — no code yet.** Decisions recorded in
> `docs/DECISIONS.md` D-024 (email ingest) + D-025 (carrier tracking). Build plan in §12; open
> questions in §13. Backed by a research pass (vendor email formats, Gmail headless auth, buyer-side
> retailer APIs, carrier aggregators) + a codebase integration audit.

## 1. Goal

Stop hand-typing purchases. When an order confirmation lands in the company Gmail from Amazon
(Business), Newegg, or Micro Center, the system should automatically draft the purchase; when the
shipping notification arrives, it should attach the carrier tracking number and follow it to
delivery — **all on-box, no retailer-account logins, operator confirms the draft.**

## 2. The core idea — an order is a *record that fills in over time*, not one email

A single purchase is assembled from **several emails over the order's life**, correlated by the
**vendor order number** (and deduped by email `Message-ID`):

| Event (email) | What it carries | Action on the purchase |
|---|---|---|
| **Order confirmation** | items, order #, (partial) costs | **Create** a draft purchase (`source_type=email`, unresolved line items) |
| **Shipping confirmation** | carrier + tracking #, which items shipped | **Attach a shipment** → the carrier poller tracks it to delivery |
| **Invoice / final charge** *(or Amazon Business API)* | authoritative subtotal / tax / shipping / total | **Enrich** the purchase's money fields |
| **Cancellation / payment declined / item unavailable** | which items dropped, why | **Flag** the line items / purchase; surface to the operator |

This is the key design decision: the ingest path is **idempotent and additive** — re-seeing an
email is a no-op, and later emails *update the same purchase* rather than creating duplicates. It
directly answers the two constraints you raised: **(a)** Amazon's confirmation lacks the full
cost breakdown → the totals get filled in later from the invoice email or the Amazon Business API;
**(b)** payment-declined / item-issue emails are first-class events that amend the order.

## 3. What already exists (reuse) vs what's new

**Reuse (~70%):**
- `SourceType::Email` enum value (`crates/domain/src/lib.rs:45`) — already a valid purchase source.
- `POST /purchases/from-payload` (`extractor.rs:create_from_payload`) — persists a §11.4 JSON object
  as a draft with unresolved line items (operator confirms). The §11.4 schema is the contract.
- The on-box **broker** (cec-llm-broker, OpenAI-style `/chat/completions`) + the extractor's
  existing wiring (`EXTRACTOR_VLM_*`) — for turning email text into §11.4 JSON.
- The deterministic vendor templates already know Amazon / Newegg / Micro Center
  (`extractor.py KNOWN_VENDORS`).
- **`crates/tracking`** (`CarrierProvider` trait + poll engine) + **`crates/poller`** (interval
  worker) + the shipment endpoints + `shipment_event` log + stop-on-delivery. Tracking is fully
  scaffolded — it just has no real provider yet (`none`/`mock` only; INV-OQ-30).

**New:**
1. An **email-ingest worker** (`crates/email-ingest`, mirrors `crates/poller`).
2. An **email→§11.4 text-extraction** path on the broker (LLM, not the receipt regex templates).
3. An **idempotent ingest/enrich endpoint** (order-keyed upsert; today the API only *creates*).
4. A real **`CarrierProvider`** (EasyPost) so the poller actually tracks.
5. *(Amazon only)* an **Amazon Business API** enrichment for the missing cost breakdown.
6. Small schema additions: `purchase.email_message_id` (dedup) + `purchase.vendor_order_number`
   correlation index + line-item/purchase status for cancellations.

## 4. Architecture

```
                          ┌─────────────────────────────────────────────┐
  company Gmail  ──IMAP──▶│  crates/email-ingest  (poll every ~5 min)    │
  (app password) :993     │  • EmailProvider trait → GmailProvider/mock  │
                          │  • classify: confirm | ship | invoice |      │
                          │    cancel/decline | marketing(drop)          │
                          │  • dedup by Message-ID; whitelist+DKIM        │
                          └───────┬──────────────────────┬───────────────┘
                                  │ email text           │ carrier+tracking#
                                  ▼                       │
                    extractor /extract-email             │
                    (broker LLM → §11.4 JSON)             │
                                  │                       │
                                  ▼                       ▼
                    POST /purchases/ingest  ── order-keyed upsert ──▶  purchase (draft)
                    (create | enrich | flag)                          + line items (unresolved)
                                  │                                   + shipment(s)
                                  ▼                                          │
                    Amazon Business API ── fill tax/ship/total            carrier poller
                    (Amazon orders only)                                  (EasyPost) ──▶ delivered
                                  │                                          │
                                  └──────────────▶  operator confirms draft in the UI  ◀──┘
```

## 5. Email-ingest worker (`crates/email-ingest`)

Mirrors `crates/poller` exactly (config from env, pg pool, `provider_from_env()`, interval loop):
- **Transport:** Gmail **IMAP with an app password** (`imap.gmail.com:993`, TLS). Chosen over OAuth
  /Gmail-API for a headless worker: minimal setup (2FA + 16-char app password), no token refresh.
  IMAP IDLE is *not* worth it (Gmail drops IDLE at 10 min) — **poll every 3–5 min** (ample for the
  shop's variable single-digits-to-tens of orders/week; orders are usually bulk = one confirmation
  with many lines). **Credentials are not in `.env`** — the worker reads every `active` Gmail account
  from the **Connections store (§5a)** and polls each, so multiple mailboxes are supported. Account
  kind is pluggable (`gmail_imap` | `gmail_oauth` | `mock`).
- **Mailbox hygiene:** read from a dedicated label (e.g. `Vendors/Processing`); on success, label
  `Vendors/Ingested` + mark read; on failure, `Vendors/Error`. This *is* the work queue and gives a
  human-visible audit trail.
- **Anti-phishing filter (important):** accept only DKIM/SPF-authenticated mail from a **sender
  whitelist**, then classify by subject/body. Lookalike domains are a real risk (`cs-newegg.com`,
  `account-verify-amazon.com`). Whitelist (to confirm during build):
  - Amazon: `order-update@amazon.com`, `auto-confirm@amazon.com`, `ship-confirm@amazon.com`,
    `shipment-tracking@amazon.com` (+ `@amazon.com` DKIM).
  - Newegg: `@newegg.com` (DKIM); **reject** marketplace-seller and lookalike domains.
  - Micro Center: `@microcenter.com`.
- **Classification:** confirmation vs shipping vs invoice vs cancel/decline vs marketing. Subject +
  body heuristics, then the LLM confirms the type. "shipped/tracking/your order has shipped" →
  shipping event (not a new purchase); promo-dominant + weak transaction data → drop.
- **Dedup:** every processed email's `Message-ID` is recorded; a unique constraint makes re-ingest a
  no-op (see §9).

## 5a. Connected accounts — multi-account credential store (frontend + API)

Accounts are **not** hardcoded in `.env`. Operators **link** them in the UI / via the API, so the
shop can add a Gmail mailbox, the Amazon Business connection, and the carrier key without a redeploy —
and run **several** mailboxes at once (the company uses a different/dedicated Gmail).

- **`connection` table (admin-managed):** `id`, `kind` (`gmail_imap` | `gmail_oauth` |
  `amazon_business` | `carrier`), `label`, `status` (`active|disabled|error`), `config` (non-secret
  JSON: host/user, carrier name, `last_polled_at`, `last_error`), and a **`secret_enc`** blob (app
  password / OAuth refresh token / API key) **encrypted at rest**.
- **Encryption at rest:** secrets are sealed with a key held only in the gitignored `.env`
  (`CONNECTIONS_SECRET_KEY`; libsodium sealed-box or `age`). Plaintext is **write-only** — accepted
  on create/rotate, **never returned** by any GET (same posture as `api_token`).
- **API (admin-gated, mirrors `/auth/tokens`):** `POST /connections` (link), `GET /connections`
  (metadata only — no secrets), `POST /connections/{id}/test` (actually connect + report), `PATCH
  /connections/{id}` (enable/disable/relabel), `POST /connections/{id}/secret` (rotate), `DELETE`.
- **Frontend:** a `/ui/connections` admin page — list connections with status + a **Test** button,
  and an "Add" form per kind (Gmail: host/user/app-password; Amazon Business: OAuth connect; carrier:
  provider + API key). Surfaces `last_polled_at` / `last_error` so a broken account is visible.
- **Consumers:** the email worker polls every `active` `gmail_*` connection; the carrier poller and
  the Amazon enrichment read their connections the same way (so the carrier key can move out of
  `.env` too).
- **Gmail linking — v1 = IMAP app password** (paste the 16-char app password; simplest). **OAuth**
  ("Sign in with Google" → store a refresh token) is a later enhancement — it needs a Google OAuth
  client + redirect flow, more frontend work than app-password entry, but avoids app passwords.

## 6. Extraction (email → §11.4 JSON)

The receipt **templates are regex for printed receipts**, not HTML emails — so emails go through the
**LLM**, not the templates. Add a text path to the **extractor service** (keep all model/prompt
logic in one place, reuse the broker wiring): `POST /extract-email {raw_email|text, vendor_hint}` →
the broker (`/chat/completions`, a **text** model) → a §11.4 object. Implementation notes from the
research:
- Prefer the **`text/plain` MIME part**; fall back to a real HTML→text conversion (not regex tag
  stripping). Strip nav/unsubscribe/footer boilerplate before the model.
- Use a dedicated **`EXTRACTOR_LLM_*`** var set (separate from `EXTRACTOR_VLM_*`) because the text
  model differs from the vision seat (e.g. `deepseek-v4-flash` vs `cec-vision-judge`).
- The model returns the §11.4 schema **plus** ingest hints: `event_type`
  (`order_confirmed|shipped|invoiced|cancelled|payment_failed`), `vendor_order_number`, and any
  `carrier`/`tracking_number`. Money stays a **decimal string** (D-023).
- The worker (Rust) calls the extractor, not the broker directly — separation of concerns; the
  worker only does email + correlation, the extractor owns extraction.

## 7. Order correlation + the idempotent ingest endpoint

Today `from-payload` always **creates**. Enrichment needs an order-keyed **upsert**. Add
`POST /purchases/ingest` taking a typed procurement event:

```
{ "event_type": "order_confirmed|shipped|invoiced|cancelled|payment_failed",
  "vendor": "...", "vendor_order_number": "112-...", "email_message_id": "<...@...>",
  "extraction": <§11.4 object>,            // for confirm/invoice
  "shipment": { "carrier": "...", "tracking_number": "..." },   // for shipped
  "cancellation": { "line_skus": [...], "reason": "payment_declined|out_of_stock|..." } }
```

Server logic (atomic, row-locked):
1. **Dedup:** if `email_message_id` already seen → `200 {duplicate:true}`, no change.
2. **Correlate:** find the purchase by `(vendor, vendor_order_number)`.
   - none + `order_confirmed` → **create** the draft (today's `persist_extraction`).
   - exists + `invoiced` → **update** money fields (subtotal/tax/shipping/total) — fills Amazon's gap.
   - exists + `shipped` → **attach a shipment** (reuse the shipment create path) → poller tracks it.
   - exists + `cancelled`/`payment_failed` → **flag** matching line items (new line status
     `cancelled`) and/or a purchase-level `intake_status`; surface to the operator.
3. Everything stays a **draft** (`resolution_status=unresolved`) until the operator confirms.

This keeps split orders sane (Amazon sends multiple order numbers → multiple purchases, optionally
linked) and partial shipments sane (one purchase, many shipments — already supported).

## 7a. Reverse flow — cancellations, returns / exchanges, serial swaps

Procurement isn't only inbound. The ingest pipeline also recognizes the **reverse** events and links
them to the **existing RMA lifecycle** — which already does the serial swap:
`POST /rma/{id}/replacement` retires the predecessor's serial and binds the replacement's new one
(and re-validates the system). So email-ingest *feeds* that flow; it doesn't reinvent it.

| Email event | `event_type` | Action (operator-confirmed unless noted) |
|---|---|---|
| Order/item **cancelled**, **payment declined**, **out of stock** | `cancelled` / `payment_failed` | Flag the line items (`line_status=cancelled`) + purchase `intake_status`; auto-applied + shown to operator |
| **Refund** confirmed | `refunded` | Suggest an RMA status → `refunded`; if a unit was received, prompt to retire/return it |
| **Return** initiated / label issued | `return_initiated` | Suggest opening/updating an RMA on the matching unit |
| **Replacement / exchange shipped** | `exchange_shipped` | Attach the replacement's shipment (tracking) **and** stage an RMA *replacement*; the **new serial is captured at physical receive/scan** (it's rarely in the email) |

The hard part is matching a reverse event to a **specific physical unit** (an order has many units;
the email rarely names a serial). So these produce **operator-confirmable suggestions**, not silent
mutations: the worker correlates by order number + product (+ an RMA number if the email carries
one), surfaces the candidate unit(s), and the operator confirms the flag / RMA action / serial swap.
The actual swap completes through the existing RMA replacement + scan, so chain-of-custody and the
append-only event log stay intact. (This is why the line-item→product mapping and the serial swap
**stay human-gated** — per your call — while tracking + cost enrichment auto-apply.)

## 8. Amazon Business API (the cost-breakdown gap)

Because Amazon's **confirmation email omits tax/shipping/total**, Amazon orders get a hybrid: the
email creates the record fast, and the **Amazon Business API** (Reconciliation / Order Reporting,
available to Amazon Business accounts — *you have one*) supplies the authoritative financials, keyed
by order number, as the `invoiced` enrichment event. Newegg/Micro Center emails are itemized enough
to skip an API. Owner believes it's the **newer Amazon Business API (OAuth)** — the design assumes
that (linked as an `amazon_business` connection, §5a). **Verify the exact access before phase 7**
(legacy Order Reporting is CSV/scheduled; the newer API is OAuth/REST); it decides the enrichment call.

## 9. Data-model changes (one migration `0007_email_ingest`)

```sql
ALTER TABLE purchase ADD COLUMN email_message_id   text;     -- dedup (RFC 2822 Message-ID)
ALTER TABLE purchase ADD COLUMN vendor_order_number text;    -- correlation key
ALTER TABLE purchase ADD COLUMN intake_status       text;    -- e.g. confirmed|partial|cancelled|payment_failed
CREATE UNIQUE INDEX purchase_email_message_id_uniq ON purchase (email_message_id) WHERE email_message_id IS NOT NULL;
CREATE UNIQUE INDEX purchase_vendor_order_uniq     ON purchase (vendor_id, vendor_order_number) WHERE vendor_order_number IS NOT NULL;
ALTER TABLE purchase_line_item ADD COLUMN line_status text;  -- ordered|cancelled|backordered
```
(Append-only migration; nullable columns so existing rows are unaffected. Final column set TBD in
review.)

## 10. Carrier tracking — wire a free-tier multi-carrier aggregator behind the existing trait (D-025)

Tracking numbers come from §5/§7 (shipping emails), **not** from logging into retailer accounts
(researched + rejected: no buyer-side APIs; scraping is ToS-violating + CAPTCHA/2FA-brittle). For the
actual polling, the cost research is decisive:

- **A fully-free *direct* carrier path is not viable.** UPS, FedEx, and DHL have free tracking APIs,
  but **USPS retired its free Web Tools (Jan 2026)** — it now requires a commercial contract (~$599/mo
  floor), and USPS is common for small parcels. Bare-number carrier auto-detect is only ~70–80%
  reliable, and it's four OAuth integrations (~2–3 weeks vs ~2–4 days for an aggregator). Rejected.
- **The free path that works = a free-tier aggregator.** **TrackingMore** gives **140 trackings/month
  free** (≈ 35 orders/week), one API key, multi-carrier auto-detect, and **polling** — which fits the
  existing `crates/poller` exactly (no webhooks needed). Grows to **$2.99/mo for 420**. → **Primary:
  TrackingMore (free tier).**
- **Alternative (push):** **EasyPost** — tracking is only free if you buy *labels* through them (you
  don't; you receive), so inbound tracking is **~$0.02–0.03 per tracker** (deduped 3 mo) ≈ $3–5/mo at
  your volume, but **webhooks are free on every tier** if you later want push instead of the 3 h poll.
- (17TRACK 100/mo, AfterShip 50/mo, Ship24 10/mo free tiers are smaller and gate webhooks behind
  paid plans; TrackingMore's 140/mo free is the most usable for this volume.)

**Wiring:** no new worker — `crates/poller` already polls active shipments; implement `CarrierProvider`
for the chosen aggregator and set `CARRIER_PROVIDER` + the key (via the Connections store §5a, or
`.env`). Start on TrackingMore's free tier; switching providers later is just another trait impl.

## 11. Operator UX & guardrails

- Every ingest yields a **draft** the operator reviews in the receipt→inventory UI (the line-item
  resolve/expand page from the backlog pairs naturally with this). Enrichment (costs, tracking) is
  applied automatically since it's additive and corrigible.
- **Confirmation ≠ received goods.** Email ingest fills the *purchase + cost + shipment* side; the
  physical receive + serial scan stays the manual step it is today (the system already separates
  these). Delivery (from the carrier poller) can prompt the "receive & scan" worklist.

## 12. Security & privacy

- **On-box LLM** (broker) — emails carry PII (names, addresses, card tails); they never leave the
  box (§11.2). Account secrets (IMAP app password, carrier key, Amazon OAuth token) live
  **encrypted in the Connections store (§5a)**, not in `.env`; the only `.env` secret is the
  Connections master key. The worker→API call uses a least-privilege **operator** bearer token.
- Worker container gets the same hardening as the others (non-root, `cap_drop: ALL`,
  `read_only` + tmpfs, mem/pids limits). Sender whitelist + DKIM/SPF defeats phishing lookalikes.

## 13. Build plan (phased — each shippable on its own)

1. **Carrier provider** — wire the chosen tracking provider (§10) so the existing poller tracks the
   shipments you already enter by hand. Smallest, immediately useful. (`crates/tracking` impl + key.)
2. **Connections store + admin UI/API** — the `connection` table + encrypted secret store +
   `/connections` endpoints + `/ui/connections` page (§5a). Foundational; lets you link the Gmail(s),
   Amazon Business, and the carrier key without `.env` edits.
3. **Email worker skeleton** — `crates/email-ingest` + `EmailProvider`/Gmail(IMAP)/mock, reads the
   `active` Gmail connections, `Vendors/*` label flow, Message-ID dedup, sender-whitelist + DKIM.
4. **Extraction** — extractor `/extract-email` (broker **text** model) → §11.4 + ingest hints.
5. **Ingest endpoint** — `POST /purchases/ingest` (create/enrich/flag) + migration `0007`.
6. **Wire forward + reverse end-to-end** — confirm→draft, ship→shipment, cancel/decline→flag, and
   the reverse flow (return/exchange/refund → RMA suggestions, §7a); test against real forwarded
   samples (Amazon/Newegg/Micro Center).
7. **Amazon Business API enrichment** — fill the cost breakdown for Amazon orders.

## 14. Open questions (need your input before/at build)

**Resolved [2026-06-28]:**
- ✅ *Mailbox:* a separate/dedicated company Gmail — linked via the Connections store (§5a),
  multi-account supported.
- ✅ *Human-in-the-loop:* line-item→product mapping **and** serial swaps stay operator-gated;
  tracking + cost enrichment auto-apply.
- ✅ *Volume:* variable — single digits to tens of orders/week, usually bulk. Poll ~5 min is plenty.
- ✅ *Carrier provider:* **TrackingMore** (free tier — 140/mo, polling, $0 at this volume). Free
  *direct* path rejected (USPS retired free tracking, Jan 2026). EasyPost is a documented later swap
  if push/webhooks are ever wanted — switching is just another `CarrierProvider` impl. (§10, D-025)
- ✅ *Gmail linking v1:* paste an **IMAP app password** in the Connections UI; Google **OAuth** is a
  later enhancement. (§5a, D-026)

**Still open (don't block phases 1–2):**
1. **Amazon Business API access** — confirm it's the newer OAuth **Amazon Business API** (assumed)
   vs legacy Order Reporting; decides the §8 enrichment call. Gates build phase 7 only.
2. **Connections encryption key** — where `CONNECTIONS_SECRET_KEY` lives (gitignored `.env`, reuse
   the age key?) and the rotation story. Needed for phase 2 (Connections store).

## 15. Decisions logged

- **D-024** — email ingest worker + order-keyed incremental-enrichment model, incl. the reverse
  flow (cancellations / returns / exchanges → RMA suggestions) (this doc §2, §7, §7a).
- **D-025** — carrier tracking behind `CarrierProvider`: **TrackingMore free tier** primary (polling
  fits the poller, $0 at this volume), EasyPost the push alternative; the free *direct* path is dead
  (USPS retired free tracking, Jan 2026). Reject retailer-account scraping (§10, INV-OQ-30).
- **D-026** — multi-account **Connections** store: operators link Gmail/Amazon/carrier accounts via
  an admin UI + API; secrets encrypted at rest, write-only; the worker/poller read enabled
  connections instead of `.env` (this doc §5a).
