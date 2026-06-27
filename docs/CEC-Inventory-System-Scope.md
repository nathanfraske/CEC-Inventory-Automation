# CEC Inventory System: Scope

Working name: CEC Inventory (placeholder, naming is yours to set)
Version: 0.4.0
Status: EXPLORATORY. No decisions locked. This is the "what you would need" pass, written before any spec commitment.
Date: 2026-06-26

Status tiers used below: LOCKED / working basis / PROPOSED / OPEN. Nothing here is above PROPOSED yet.

---

## 1. Scope and intent

Standalone shop inventory system. Takes a receipt (phone photo, PDF, digital, email, including multi-image receipts stitched automatically), turns it into structured purchase records with quantities, serials, and purchase price, attaches the receipt image and all RMA-relevant metadata to the parts, polls shipment tracking for online orders, and tracks what is in the building. Parts carry an owner (shop vs customer). Assembled machines are tracked as systems that carry a two-layer warranty (manufacturer plus a CEC-provided warranty, full or refurb), a validation state that re-validates after a change, and an ownership-transfer path with a documented parts sweep. RMA is handled across three execution modes including the case where the customer keeps the part and files the RMA with CEC assisting.

Out of scope for now, seam defined in Section 19: the cec.direct build-queue integration. Build it later, do not couple to it now.

The intent word "maximum efficiency" resolves to three concrete things:
1. Known vendors parse deterministically and instantly (template path, no model, no GPU). Unknown receipts fall back to a vision model. You do not pay model latency for a Newegg or DigiKey receipt you buy from every week.
2. Serial capture is a camera scan loop, not typing, and where the vendor prints the serial (Micro Center) it collapses to a one-tap verify.
3. A long receipt is captured in pieces and assembled automatically, and online orders track themselves to the door without manual checking.

---

## 2. Facts that drive the model (read this before the schema)

Five facts drive every table:

**A. Serials are vendor-dependent, mostly absent.** Most receipts carry only a vendor description string, sometimes a vendor SKU, a quantity, and a price, with the serial on the box and nowhere on the paper. There the serial is captured separately from the part and creates the unit: two steps, one record. But some vendors print the serial on the receipt for serialized items. Micro Center does this, per line: the GPU line carries its serial, the cable line does not. When the receipt carries the serial, extraction pre-populates the unit and the physical scan downgrades to a verification pass (Section 13.4). Either way the structure is identical: the line item is the lot, the unit is the thing, the serial binds to the unit.

**B. Quantity N on a line means N distinct physical things.** "RTX 4090 x4" on a receipt is one line item and four units, each with its own serial, its own RMA clock, its own status. RMA is per unit. The line item is the lot; the unit is the thing.

**C. Not everything is serialized.** A GPU, PSU, board, CPU, RAM kit, SSD, and a CEC module each get a unique serial and a unit row. Cables, screws, thermal paste, and bulk passives do not. They are quantity-on-hand stock. The model needs both a serialized-unit path and a bulk-stock path, and a per-category policy for which is which.

**D. Parts have an owner, and ownership splits the RMA path.** A custom build mixes two pools: parts CEC bought and owns, and parts the customer supplied. If a shop-owned part fails, CEC's receipt and warranty govern. If a customer-supplied part fails, the customer's receipt and warranty govern and CEC can only assist. Trade-ins resolve to shop-owned. Every unit carries `owner [shop | customer]` and the RMA execution path (Section 7) reads off it.

**E. A delivered system is the unit of ownership, warranty, and transfer.** Parts are bought, stocked, and RMA'd individually, but they reach a customer as an assembled system, and that system is what the customer owns, what carries CEC's provided warranty, and what transfers on resale. A system has a validation state that a change (RMA, swap) breaks and a re-validation restores. Ownership, the CEC warranty clock, re-validation, and transfer attach to the System (Section 6); the manufacturer warranty and the serial stay on the unit.

This is the standard catalog / purchase / lot / unit split, with an ownership axis and a system grouping. Getting the schema right in Phase 0 is the whole game; everything else hangs off it.

---

## 3. Functional pipeline

```
  ingest          extract            resolve identity       capture            store
  -------         -------            ----------------       -------            -----
  receipt   -->   line items   -->   map to catalog    -->  scan serials  -->  units +
  (img/pdf/       (vendor,            SKU (create or          per physical      bulk stock,
   email/         date/time,          confirm match;          unit, or          receipt files
   manual/        items, prices,      expand bundles)         verify if         attached,
   opening;       serials,                                    on receipt        events logged
   stitched       shipments)
   if multi)
```

1. **Ingest.** Normalize any input to image (rasterize PDF pages) or extractable text. A receipt too long for one frame is assembled first (Section 10): whole-page capture for web, guided overlapping capture plus stitching for paper. Email path: a forwarding address or IMAP poll, not a cloud connector, to keep self-host parity (Section 18). Opening-balance and trade-in are no-receipt intakes (Sections 8 and 9).
2. **Extract.** Receipt to structured JSON line items + per-field confidence, with serials where the vendor prints them, bundle lines flagged for expansion, and shipment tracking handles where present. Hybrid engine, Section 11.
3. **Resolve identity.** Map each line's description/SKU to a canonical Product. First purchase creates the catalog row (operator confirms). Bundle lines expand into their component products.
4. **Capture.** Each serial scan creates a unit bound to its line item, or verifies a receipt-supplied serial. Bulk items increment quantity-on-hand. Each unit gets an internal asset tag (Section 13.5).
5. **Store.** Units and bulk stock persisted, receipt files in object storage, every mutation written to the unit event log (Section 16).

In parallel for online orders, a shipment poll runs from order time to delivery (Section 12), logging carrier status into the order and, after receiving, the resulting parts' history. Assembly, delivery, re-validation, and transfer are separate flows that operate on the stored units once they exist (Section 6).

---

## 4. Data model (PROPOSED)

Entities and the fields that matter. Postgres. JSONB for raw extract payloads, event detail, validation snapshots, and carrier payloads. Timestamps are `timestamptz`.

**Vendor** (place of purchase)
`id, name, address, website, rma_url, rma_contact, account_number, notes`

**VendorReturnPolicy** (return windows differ by vendor and category)
`id, vendor_id, category (nullable for default), return_window_days, doa_window_days, restocking_fee_pct, notes`

**Manufacturer**
`id, name, rma_url, rma_contact, warranty_policy_url, default_warranty_months, replacement_warranty_days (the term a swap carries, commonly 90), warranty_basis_default [original_remainder | replacement_term], warranty_transferable (bool, false where the maker voids warranty on resale or for a non-original purchaser), warranty_start_basis [purchase_datetime | manufacture_date], notes`

**Product** (catalog / SKU, the canonical part identity)
`id, manufacturer_id, model, mpn, upc_ean, category, serialized (bool), default_warranty_months (override of manufacturer default), serial_format_regex (nullable, for OCR-confusion validation), datasheet_url, notes`

**Purchase** (one receipt / order / intake)
`id, vendor_id (nullable for opening-balance with unknown origin), purchase_datetime (date and time, the manufacturer-warranty start basis), order_number, invoice_number, currency, subtotal, tax, shipping, discount_total, total, payment_method, source_type [physical_photo | pdf | email | manual | trade_in | opening_balance], receipt_files[] (object-store refs, the stitched artifact plus original segments), raw_extract (jsonb), extract_confidence, created_by, created_at`

**PurchaseLineItem** (the lot)
`id, purchase_id, product_id (nullable until resolved), description_as_printed, vendor_sku, quantity, unit_price, line_total, currency, is_bundle (bool), parent_line_id (nullable, set on expanded bundle components), allocated_landed_cost (computed, Section 14), resolution_status [unresolved | suggested | confirmed]`

**Shipment** (one tracked parcel for an online order, Section 12)
`id, purchase_id, carrier [usps | ups | fedex | dhl | other], tracking_number, tracking_url, status [pre_transit | label_created | in_transit | out_for_delivery | delivered | exception | returned | unknown], expected_delivery_date, shipped_at, delivered_at, last_polled_at, poll_state [active | stopped], line_item_ids[] (nullable, for split shipments), notes`

**ShipmentEvent** (append-only carrier status history)
`id, shipment_id, event_status, carrier_description, location, occurred_at (carrier timestamp), polled_at (when fetched), raw (jsonb)`

**InventoryUnit** (one serialized physical thing)
`id, product_id, line_item_id (nullable for trade-in / opening-balance / unknown origin), system_id (nullable, set when built into or delivered as part of a system, Section 6), owner [shop | customer], customer_ref (nullable, set when owner=customer or for field-installed units), serial_number, serial_source [receipt | scan | ocr | manual], verified (bool, true once a physical scan confirms a receipt-supplied serial), asset_tag (internal ID, Section 13.5), condition [new | open_box | used | refurb | unknown], acquisition_method [purchase | trade_in | rma_replacement | gift | salvage | opening_balance], status [in_stock | reserved | in_build | installed | with_customer | shipped | rma_open | pending_return | defective | returned | scrapped], location_bin (nullable when with_customer), unit_cost (landed, allocated), mfr_warranty_expires (computed, Section 5), mfr_warranty_basis [standard | original_remainder | replacement_term | registered], cec_warranty_class [full | refurb | none], cec_warranty_start (timestamptz, set at delivery, Section 6.2), cec_warranty_expires (computed, Section 5), replaces_unit_id (nullable, set on rma replacements), registered (bool), rma_eligible (bool, computed, overridable), rma_block_reason, photos[] (part / serial sticker / box), notes, intake_at, intake_by`

Build linkage is reached through the unit's System (`system_id -> System.build_id`), so the unit does not carry a separate build foreign key.

**StockItem** (bulk, non-serialized: cables, screws, paste, passives)
`id, product_id, location_bin, asset_tag (bin label), quantity_on_hand, reorder_point, cost_basis (moving_avg or per-lot, OPEN INV-OQ-8), notes`

**Kit / bundle handling.** Two shapes break the line-equals-N-identical-units rule:
- A RAM kit is one Product (the kit SKU) with one box serial in the common case, optionally N component serials. Default: kit is a single unit by its box serial, with an optional `component_serials[]` sub-list. Parent/child component units only where you actually track and RMA sticks individually (INV-OQ-18).
- A combo deal is one receipt line at a combined price spanning different products (CPU + board combo). The line is marked `is_bundle = true` and expands into child PurchaseLineItems, one per component Product, each with an allocated price (INV-OQ-17). Each child then spawns its own units.

**System** (the as-built / as-delivered machine, Section 6)
`id, label (build number / system serial), asset_tag (scannable system tag), build_id (nullable, ref to cec.direct build, Section 19), current_owner [shop | customer], customer_ref (nullable), status [in_build | in_stock | delivered | in_service | rma_open | retired], delivery_datetime (nullable, set at first customer delivery; CEC-warranty start basis), cec_warranty_class (system default for new parts), validation_state [validated | invalidated | pending_revalidation], last_validated_at, last_validated_by, notes`

**CecWarrantyPolicy** (CEC-provided warranty terms by class)
`id, warranty_class [full | refurb], category (nullable for default), term_months, transferable (bool), reset_on_transfer (bool), clock_pauses_when_invalidated (bool), notes`

**SystemValidation** (validation / re-validation / parts-sweep records, append-only, Section 6.4 and 6.5)
`id, system_id, validation_type [eol | post_change | periodic | pre_transfer | sweep], trigger [build_complete | rma | parts_swap | service | transfer_request | audit], performed_at, performed_by, result [pass | fail], parts_snapshot (jsonb: unit_id + serial at validation time), reconciliation (jsonb, when a sweep: per-unit matched | serial_mismatch | missing | unexpected_extra; overall clean | discrepancies), evidence_refs[] (test reports, captures), notes`

**SystemTransfer** (ownership transfer customer -> customer, Section 6.5)
`id, system_id, from_owner_ref, to_owner_ref, transfer_datetime, performed_by, sweep_id (ref to the authorizing SystemValidation of type pre_transfer/sweep), mfr_warranty_outcome (jsonb: per-part carried | void_non_transferable), cec_warranty_outcome [carried | reset | prorated | declined], cec_transfer_fee, result [completed | blocked_on_sweep | partial], notes`

**TradeIn**
`id, customer_ref, trade_date, source_notes, proof_of_purchase_status [provided | customer_has_will_send | customer_lacks | none], proof_files[], unit_ids[]`

**RmaCase** (the lifecycle, Section 7)
`id, unit_id, owner_at_failure [shop | customer], party [vendor | manufacturer], execution_mode [cec_managed | customer_ships_to_cec | customer_managed_assist], proof_source [cec_receipt | customer_receipt], custody [at_cec | with_customer | in_transit_to_cec | in_transit_to_vendor | in_transit_to_customer], rma_number, fault_description, status [open | info_provided_to_customer | awaiting_customer_action | shipped_to_vendor | awaiting_replacement | replacement_received | replacement_with_customer | refunded | denied | closed], assist_artifacts (jsonb: proof_package_ref, portal_link, contact, instructions_sent_at), advance_replacement (bool), auth_hold_ref (nullable), return_due_date (nullable), opened_at, closed_at, shipped_at, return_tracking, replacement_unit_id (nullable), resolution, notes`

**UnitEvent** (append-only audit and provenance log, Section 16)
`id, unit_id, event_type [intake | status_change | serial_edit | verify | reserve | install | deliver | ship | location_change | owner_change | warranty_registered | revalidated | transfer | rma_open | rma_update | replace_out | replace_in | scrap | note], from_value, to_value, actor, at, system_id (nullable), rma_case_id (nullable), detail (jsonb)`

Relationships: Vendor 1..N Purchase 1..N PurchaseLineItem 1..N InventoryUnit. Purchase 1..N Shipment 1..N ShipmentEvent; a Shipment optionally references specific PurchaseLineItems for split shipments. PurchaseLineItem self-references for bundle parent/child. Product 1..N PurchaseLineItem / InventoryUnit / StockItem. Manufacturer 1..N Product. Vendor 1..N VendorReturnPolicy. System 1..N InventoryUnit (system_id), 1..N SystemValidation, 1..N SystemTransfer, and System.build_id references the cec.direct build. CecWarrantyPolicy referenced by class for term lookup. TradeIn N..N InventoryUnit. RmaCase N..1 InventoryUnit, with `replacement_unit_id` and `replaces_unit_id` chaining a unit to its successor. UnitEvent N..1 InventoryUnit.

Receipt-to-part reachability: `unit -> line_item -> purchase -> receipt_files[]`, and `purchase -> shipment -> shipment_event` for the in-transit history. Units also carry their own `photos[]`. Proof of purchase for RMA lives on the Purchase (or the TradeIn for traded parts, or the customer's records for customer-owned), reachable from the unit.

---

## 5. Ownership, the two warranty layers, and RMA readiness (PROPOSED)

### 5.1 Ownership

`owner [shop | customer]` per unit. Shop at intake, flipped to customer at delivery (Section 6.2). Customer-supplied parts are `owner = customer` from intake. Trade-ins resolve to `owner = shop`.

### 5.2 Two warranty layers

Every part a customer holds carries two warranties on two different clocks:

1. **Manufacturer warranty (their parts warranty).** What the part maker covers. Start = `Purchase.purchase_datetime` (when CEC bought the part) or the manufacture date, per `Manufacturer.warranty_start_basis`. Term = `Product.default_warranty_months` (or the manufacturer default). Basis = `mfr_warranty_basis [standard | original_remainder | replacement_term | registered]` (RMA replacements and registration adjust it, Sections 7.6 and 7.9). Field: `mfr_warranty_expires`. Transferable per `Manufacturer.warranty_transferable`.

2. **CEC-provided warranty (our provided warranty).** What CEC covers for the customer on top of the maker. Start = `cec_warranty_start` = the system `delivery_datetime` (Section 6.2), a different moment than the manufacturer start. Class = `cec_warranty_class [full | refurb | none]`; `none` means CEC does not warrant this part (customer-supplied, or out of CEC coverage). Term by class from `CecWarrantyPolicy` (full and refurb terms are CEC policy, INV-OQ-23). Field: `cec_warranty_expires`.

The two starts being different is the point. The manufacturer clock began when CEC acquired the part; the CEC clock begins at handoff to the customer. "Time left on their parts warranty vs our provided warranty" is therefore two computations, shown side by side per part and rolled up per system (Section 6.3).

Example per-part view:
```
RTX 4090   serial GPU-2291X   condition new
  MFR (their parts)   full     22 mo left   expires 2028-04-12  (started 2026-03-02, CEC purchase)
  CEC (our provided)  full     11 mo left   expires 2027-06-01  (started 2026-06-01, delivery)
```
And a refurb part in the same system:
```
PSU 850W   serial PSU-7741   condition refurb (RMA replacement)
  MFR (their parts)   replacement_term   2 mo left   expires 2026-08-26
  CEC (our provided)  refurb             3 mo left   expires 2026-09-01
```

### 5.3 Full vs refurb class

`cec_warranty_class` is per part because a system mixes new and refurb. A new part delivered new gets `full`. A refurb part (condition `refurb`, or an RMA replacement that returned refurbished) gets `refurb`, which `CecWarrantyPolicy` maps to a shorter term. The system view surfaces the mix so the customer sheet shows which parts are full-warranty and which are refurb.

### 5.4 Computations

- `mfr_warranty_expires` = warranty start (per basis) + term, with the replacement and registered cases overriding (Sections 7.6, 7.9).
- `cec_warranty_expires` = `cec_warranty_start` + `CecWarrantyPolicy` term for the class. Whether re-validation resets this start is policy (INV-OQ-24, INV-OQ-27).

### 5.5 RMA readiness and CEC coverage are two different states

- `rma_eligible` (the external RMA path) is true when the unit has a serial, proof of purchase exists, identity is resolved, and `mfr_warranty_expires >= today`. Block reasons:

| Condition | rma_block_reason |
|---|---|
| No serial captured | `no_serial` |
| Trade-in, customer will send proof | `awaiting_proof_from_customer` |
| Trade-in or customer-owned, no proof anywhere | `no_proof_of_purchase` |
| `mfr_warranty_expires < today` | `warranty_expired` |
| Product not resolved | `identity_unresolved` |

  Ownership does not block; it sets the execution path (Section 7). A `customer`-owned part with valid customer proof is `rma_eligible = true` but defaults to customer-managed: CEC assists, CEC does not file on its own proof.

- `cec_warranty_active` (will CEC service it under its own warranty) is true when `cec_warranty_class != none`, `cec_warranty_expires >= today`, and the unit's System is `validation_state = validated`. The validation condition is the link in Section 6.4: a change suspends CEC coverage until re-validated.

---

## 6. Systems: delivery, re-validation, and warranty transfer (PROPOSED)

### 6.1 The System entity

A System is the as-built/as-delivered machine (Section 4). It groups units (`unit.system_id`), references the cec.direct build (`System.build_id`, Section 19), and carries `current_owner`, `customer_ref`, `delivery_datetime`, the system CEC warranty class, and `validation_state`. It moves `in_build -> in_stock (built, unsold) -> delivered -> in_service`, with `rma_open` and `retired` as needed. A system gets its own scannable asset tag like a unit (INV-OQ-29).

### 6.2 Delivery: shop to customer, and the CEC warranty start

Delivery is the first ownership transfer and the moment that documents when the parts become the customer's. The action:
- Sets `System.current_owner = customer`, `customer_ref`, `delivery_datetime = now`.
- For every member unit: `owner = customer`, `customer_ref`, `cec_warranty_start = delivery_datetime`, `cec_warranty_class` set (`full` default, `refurb` for refurb parts), and `cec_warranty_expires` computed.
- Requires the system to be `validated` at handoff (the build passed EOL, Section 6.4), so coverage is live on delivery.
- Snapshots the manufacturer warranty remaining per part at handoff for the customer sheet.
- Logged as `deliver` and `owner_change` UnitEvents and a SystemValidation of type `eol`.

This is distinct from a customer-to-customer transfer (6.5). Delivery starts the CEC clock; transfer moves an existing system to a new owner.

### 6.3 The two warranty clocks, side by side

The documentation surface: per part, the manufacturer remaining vs the CEC remaining with class and expiry (Section 5.2 example). Per system, a rollup: the CEC warranty summary (earliest CEC expiry among covered parts, or the system policy term), the manufacturer-expiry spread, and the full-vs-refurb-vs-uncovered breakdown. This is the customer-facing warranty sheet and the shop's at-a-glance coverage state.

### 6.4 Re-validation after a change

`validation_state [validated | invalidated | pending_revalidation]` on the System is the warranty gate.

- Any membership change (RMA replacement, parts swap, add or remove a unit) sets `invalidated` and, per Section 5.5, suspends `cec_warranty_active` on the system's units until re-validated.
- A re-validation pass is a SystemValidation of type `post_change`: CEC re-runs its bench/EOL check and, on `pass`, sets `validated`, `last_validated_at/by`, and restores coverage. Documented with the parts snapshot, result, and evidence refs.
- The re-validation can consume the platform's existing EOL pass/fail and the per-account golden comparison (the platform's local gate is computable on the Hub and bench offline), rather than reinventing a test. Inventory records the result; it does not re-implement the gate. The minimum pass criteria after a change is INV-OQ-25.
- Whether the CEC clock pauses during the invalidated window or runs continuously is policy (INV-OQ-24). Default proposal: the clock runs continuously (calendar-based, expiry date fixed) and coverage is suspended (no claims) until re-validated, so delaying re-validation costs coverage time but does not move the expiry. `CecWarrantyPolicy.clock_pauses_when_invalidated` carries the chosen rule.

This is the mechanism for "re-validate the system to re-validate the warranty": the change breaks `validated`, the documented re-validation restores it, and `cec_warranty_active` follows.

### 6.5 Warranty transfer with a documented parts sweep

Resale is an ownership transfer customer to customer. Precondition: a documented parts sweep proving the system is intact.

- **Parts sweep.** Scan every unit in the system and reconcile the scanned serial set against the recorded set for that system. Per-unit result: `matched | serial_mismatch | missing | unexpected_extra`. Overall: `clean | discrepancies`. Recorded as a SystemValidation of type `sweep` (or `pre_transfer`) with the full reconciliation. Same scan-and-reconcile primitive as the receipt verification pass (Section 13.4), here against system membership rather than a single receipt line.
- **Clean sweep authorizes the transfer.** Discrepancies block it or route to resolution (INV-OQ-28): a swapped part means the warranty does not carry for that part, or the changed part is brought into the record and the system re-validated. CEC does not transfer a warranty on a system modified out from under it; the sweep is the proof of integrity.
- **On transfer (clean sweep):** `System.current_owner = new customer`; member units' `customer_ref` updated; a SystemTransfer record links the authorizing sweep; `transfer` and `owner_change` events logged.
- **Warranty on transfer:**
  - Manufacturer: carries with the serial and the remaining time if `Manufacturer.warranty_transferable`; non-transferable makers (warranty tied to the original purchaser) flag those parts void-on-transfer (INV-OQ-26).
  - CEC-provided: per `CecWarrantyPolicy.transferable` and `reset_on_transfer`, with an optional transfer fee (INV-OQ-27). Default proposal: on a clean sweep the CEC warranty carries the remaining term to the new owner with no reset (the system is proven intact), subject to a fresh re-validation if the last one is stale.

The sweep doubles as a re-validation trigger when it surfaces discrepancies or when the last validation is old.

---

## 7. RMA lifecycle and execution modes (PROPOSED)

RMA is the stated driver, so it gets a real lifecycle, not just a readiness flag. The case opens on a failed unit and runs through one of three execution modes.

### 7.1 The three execution modes

1. **CEC-managed (in-house).** The unit is at CEC. CEC files with the vendor or manufacturer on CEC's proof, ships it back if a return is required, receives the replacement, and intakes it. For shop-owned parts CEC holds.
2. **Customer ships to CEC, CEC runs it.** Sometimes CEC asks the customer to ship the failed part back, then handles the RMA end to end exactly as mode 1. Used when the part failed in the field but CEC wants to own the process. `custody` walks `with_customer -> in_transit_to_cec -> at_cec`.
3. **Customer-managed, CEC assists.** CEC does not take the unit. The customer keeps the part and files the RMA themselves; CEC supplies what they need and tracks it. The unit never enters CEC inventory and the replacement goes to the customer. This is the common case for a part already installed in a customer's machine.

### 7.2 Ownership x proof matrix

| Owner | Proof source | Who can file | Default mode |
|---|---|---|---|
| shop | cec_receipt | CEC, or customer with CEC-supplied proof | 1, 2, or 3 |
| customer | customer_receipt | customer (CEC assists) | 3, occasionally 2 with authorization |

The load-bearing case: a shop-owned part that failed in a customer's field machine. CEC's receipt is the proof even though the part is physically with the customer, so mode 3 requires CEC to hand the customer a proof artifact. That artifact is Section 7.4.

### 7.3 Custody and status

`custody` tracks where the physical unit is, independent of who is executing. A mode-3 unit is `with_customer` for the whole case and is not a held CEC asset, so it does not occupy a bin and its inventory status reflects the obligation, not stock on hand. Modes 1 and 2 move custody through CEC and on to the vendor. Status transitions are logged as UnitEvents (Section 16).

### 7.4 Proof-of-purchase package (the enabling feature)

One action on any unit bundles its receipt image, serial, purchase datetime, vendor, order/invoice number, and the manufacturer warranty terms into a single shareable PDF or link. CEC hands it to the customer for a mode-3 filing, or attaches it to its own filing in modes 1 and 2. This is what makes customer-managed RMA on a shop-owned part work: the customer presents CEC's proof.

### 7.5 Customer-managed assist flow

1. A unit fails in the field (installed in a customer system, or a customer-supplied part).
2. CEC chooses mode 3: help the customer file rather than take the unit.
3. CEC generates the proof package (7.4), looks up the manufacturer or vendor RMA portal and contact, drafts the fault description, and sends the customer the package, the link, and the steps. Recorded in `assist_artifacts`.
4. Case status walks `info_provided_to_customer -> awaiting_customer_action`. The unit stays `with_customer`.
5. Customer files and completes. CEC tracks to `replacement_with_customer` or `refunded` / `denied`.
6. If a replacement returns with a new serial and CEC learns it (INV-OQ-21), CEC creates the replacement unit and updates the system membership so provenance stays accurate (7.7). If CEC never learns it, the system record is flagged stale.

### 7.6 Replacement intake and remainder-of-term warranty

When a replacement arrives (modes 1 and 2, or mode 3 routed through CEC):
- A new InventoryUnit is created, `acquisition_method = rma_replacement`, `unit_cost = 0` (or the RMA fee), `replaces_unit_id` set to the failed unit, and the RmaCase `replacement_unit_id` set to it.
- Manufacturer warranty follows `mfr_warranty_basis`: original remainder or a replacement term, not a fresh full term. Default per manufacturer, override per unit.
- CEC warranty: if the replacement is refurbished, `cec_warranty_class = refurb` (refurb term); if a new replacement, `full`. The class is set on intake into the system.
- The predecessor unit moves to `returned` or `scrapped`, lineage preserved through the `replaces`/`replacement` links and the event log.

Advance replacement / cross-ship: both old and new units are briefly live. A card authorization is held (`auth_hold_ref`), the old unit is `pending_return` with a `return_due_date`, and if it is not shipped back inside the window the auth is charged. Surfaced as a due-date alert.

### 7.7 Replacement changes the system, so it re-validates

A replacement unit joins the failed unit's System (`system_id` inherited), which is a membership change. Per Section 6.4 that sets the System `invalidated` and suspends `cec_warranty_active` until a documented re-validation pass. The replacement also inherits the build linkage through the system, keeping the build-to-serial record correct, which is the exact input the platform's OQ-40 and OQ-44 want. When the customer ran the RMA and the new serial is unknown, the system carries a `provenance_stale` flag until a serial is recovered.

### 7.8 Vendor return window vs manufacturer RMA

A failed part has two exits with different windows and outcomes:
- Back to the vendor, inside the DOA or general return window (VendorReturnPolicy, typically 15 to 30 days, category-dependent). Usually a refund or a swap, and faster.
- To the manufacturer, after the vendor window closes, on the manufacturer warranty. A replacement.

At fault time the system compares `now` against `purchase_datetime + VendorReturnPolicy.return_window_days` (and `doa_window_days`) and prompts the decision: a DOA part still inside Micro Center's window goes back to Micro Center for a refund instead of a slower manufacturer RMA. `RmaCase.party` records which exit was taken.

### 7.9 Warranty registration

Some brands extend the term on registration. Registration is a tracked action: `registered = true`, a `warranty_registered` UnitEvent, and an extended `mfr_warranty_expires` override. Ties to INV-OQ-11.

---

## 8. Trade-in intake (PROPOSED)

When `source_type = trade_in`, the form does not assume a receipt exists. It resolves the unit to `owner = shop` and presents the RMA-required checklist, asking the operator to fill what is available:

- serial number (scan, type, or OCR off a photo)
- manufacturer + model (resolve to catalog, or create)
- condition
- proof of purchase: one of
  - **provided** -> capture the customer's receipt image, original purchase datetime, original vendor. Unit becomes RMA-eligible subject to warranty.
  - **customer has it, will send** -> unit `rma_eligible = false`, reason `awaiting_proof_from_customer`, flagged for follow-up. Flips to eligible when proof lands.
  - **customer lacks it / none** -> unit `rma_eligible = false`, reason `no_proof_of_purchase`. The part is still tracked, still has a serial and a location, and is marked not-RMA-able with the reason attached.

The operator is never blocked from taking a part in. The system records exactly what is missing and whether it is recoverable.

---

## 9. Opening-balance intake (PROPOSED)

Day one, CEC already owns stock with no receipt in the system. The migration on-ramp is `source_type = opening_balance` on a synthetic Purchase: serial known (scan), origin reconstructed best-effort (datetime, vendor, cost) or marked unknown. This is structurally the trade-in no-receipt case: opening-balance units with unknown origin are `rma_eligible = false` (`no_proof_of_purchase`) until reconstructed. It exists so existing stock enters the system cleanly instead of being back-filled as fake purchases.

---

## 10. Receipt capture and stitching (PROPOSED)

A receipt longer than one frame is captured in pieces and assembled into one logical receipt before extraction, automatically, for both physical paper and web receipts. Two paths because the inputs differ.

### 10.1 Web receipts: prefer whole-page capture

A web order page or confirmation is best captured whole, not as screenshots. A full-page (full-height) screenshot or a print-to-PDF gives the entire receipt in one artifact with a selectable text layer, which routes straight to the deterministic text fast-path (Section 11.1): no OCR, no stitching, no model. This is the highest-fidelity, lowest-effort path and is the default offered for web orders. Email confirmations are equally clean (full text) and go through the email ingest path (Section 3) with no stitching. The screenshot-stitch fallback (10.3) is only for when sole screenshots exist, such as a mobile app order page or a forwarded screenshot set (INV-OQ-32).

### 10.2 Physical receipts: guided overlapping capture

A long paper receipt is photographed in overlapping segments top to bottom. The capture UI guides it: after each frame it shows a ghost of the previous frame's bottom strip so the operator aligns the next frame's top to it, guaranteeing both overlap and order. An overlap of roughly 20 to 30 percent gives the stitcher reliable correspondence. Each frame is rectified first (deskew and dewarp to correct perspective and curl) so the stitch composes flat segments rather than trapezoids.

### 10.3 Stitching engine

- **Screenshots** (uniform width, pure vertical translation, no rotation or scale): deterministic. Detect the overlap by matching pixel rows between the bottom of frame N and the top region of frame N+1 (normalized cross-correlation on the overlap band), then concatenate with the duplicate band removed. Fast and reliable because there is no perspective.
- **Photos** (perspective, rotation, scale, glare, thermal fade): feature-based. OpenCV ORB feature detection and matching across the overlap, estimate the transform (translation-dominant with small rotation for a flat receipt), warp and composite, blend the seam. `cv2.Stitcher` in SCANS mode or a custom translation-dominant compositor. Low-texture whitespace bands are the failure risk; the guided overlap (10.2) keeps enough unique text per band to match.
- **Overlap de-duplication is mandatory** either way: the duplicated region between frames is removed exactly once so line items in the overlap are not double-counted downstream.

### 10.4 Degrade, do not fail

When a photo stitch cannot find a confident transform (faded thermal, heavy glare), do not block. Keep the ordered, rectified segments as a multi-page PDF and feed them to the VLM as ordered pages (Section 11.2), which extracts across pages; the merge de-duplicates the overlap from the page texts. A stitch failure degrades to multi-page handling, not a dead end. The confidence threshold for giving up on the pixel stitch is INV-OQ-33.

### 10.5 The stored artifact

The output is one stitched image or a multi-page PDF, stored as the Purchase `receipt_files` artifact (the human-viewable proof for RMA, Section 17) and the input to extraction. The original segments are kept too, so a bad stitch can be re-run without re-capturing.

Where it runs: image processing in the Python extraction service (OpenCV), invoked before extraction. The Rust backend uploads the ordered segments; the service returns the assembled artifact plus the structured extraction.

---

## 11. Receipt extraction (PROPOSED)

Hybrid engine, two paths, one output schema. This is the efficiency lever.

### 11.1 Fast path: template / rule parsers (no model)

For the vendors the shop buys from repeatedly (Newegg, Amazon, Micro Center, Mouser, DigiKey, LCSC, JLCPCB, others that recur), deterministic parsers extract line items with no GPU and no hallucination risk. `invoice2data` is the open template engine; per-vendor templates stay predictable because layouts are stable. Digital receipts with a real text layer, including the whole-page web captures from Section 10.1, go straight here. A vendor that prints serials in a stable format (Micro Center) is a high-value template target precisely because the template can pull the serial reliably (INV-OQ-5).

### 11.2 Fallback path: vision-language model

For arbitrary or first-seen receipts, and for the ordered-pages fallback from a failed stitch (Section 10.4), a VLM reads the image(s) and emits the same JSON. 2026 open-weight options, by fit:

| Model | Note |
|---|---|
| Qwen2.5-VL-7B-Instruct | Strong zero-shot receipt / document structured extraction, ~7B, accepts multiple images as ordered pages. Runs on the Jetson Orin quantized (AWQ/GPTQ, tight on 8GB, matching your Orin 7-8B sizing) or comfortably on the K3 (32GB). Default candidate. |
| dots.ocr, DeepSeek-OCR | Purpose-built OCR-to-structure models, lighter, tested well on skewed receipts in current 2026 comparisons. Lower hallucination surface than a general VLM. |
| PaddleOCR PP-Structure + small text LLM | Two-stage: layout + text, then a text model structures it. Most deterministic, most pipeline. Good if VLM hallucination on prices is a problem. |
| GLM-4.5V, DeepSeek-VL2 | Larger general VLMs, heavier. K3-class only. |

### 11.3 Service shape

The extractor is a Python service (FastAPI), Python isolated to ML per your standing split. It serves the templates and the VLM (vLLM, llama.cpp, or Ollama backing the model), and the OpenCV stitching pre-step (Section 10), and runs on the inference box (Jetson Orin or K3), not the web host. The Rust backend POSTs ordered images or text and receives the assembled artifact plus JSON. Same HTTP-endpoint seam you already use for the framework consuming inference.

### 11.4 Output schema (both paths emit this)

```json
{
  "vendor": "string",
  "purchase_datetime": "YYYY-MM-DDTHH:MM:SS",
  "order_number": "string|null",
  "invoice_number": "string|null",
  "currency": "USD",
  "line_items": [
    {"description": "string", "vendor_sku": "string|null",
     "quantity": 1, "unit_price": 0.00, "line_total": 0.00,
     "serials": ["string"],
     "is_bundle": false,
     "confidence": 0.0}
  ],
  "shipments": [
    {"carrier": "string|null", "tracking_number": "string|null",
     "tracking_url": "string|null", "expected_delivery_date": "YYYY-MM-DD|null"}
  ],
  "subtotal": 0.00, "tax": 0.00, "shipping": 0.00, "discount_total": 0.00, "total": 0.00,
  "field_confidence": {"vendor": 0.0, "total": 0.0, "datetime": 0.0}
}
```

`purchase_datetime` captures the time when the receipt prints it (point-of-sale receipts usually do); fall back to date-only at midnight local when no time is present, flagged low-confidence on the time component. `serials` is populated only when the receipt prints them. A serialized line yields one serial per unit of quantity, and the partial case (quantity 4, two serials printed) is flagged. `is_bundle` flags a combo line for expansion (Section 15). `shipments` carries any carrier, tracking number, tracking link, and shown delivery estimate parsed from an order confirmation, seeding Section 12; empty for in-store receipts.

Honest limit: for most vendors, extraction gives purchase facts but not serials or reliable MPNs, and the serial comes from the scan (Section 13). For vendors that print serials, extraction pulls them, subject to a confirming scan. Low-confidence fields surface for operator confirmation rather than committing silently.

---

## 12. Order tracking and shipment polling (PROPOSED)

For online orders, the system captures the shipment tracking and polls the carrier for status until delivery, logging every update into the order's and the resulting parts' history.

### 12.1 Getting the tracking handle

Carrier, tracking number, and tracking URL come from the order confirmation email or page (extraction, Section 11.4 `shipments`), the vendor order page, or manual entry. An order can have several shipments (split shipment), each its own tracking number covering a subset of line items, so tracking is per-shipment, linked to the Purchase, and a Shipment may reference the specific line items in its box.

### 12.2 Polling

A background worker polls active shipments on a cadence and writes each status change. Cadence: a few hours while in transit, tightening near the expected delivery date; stop polling once `delivered` or `returned`; surface exceptions (failed delivery, held, damaged) immediately. Respect carrier rate limits and back off on errors (INV-OQ-31). Statuses normalize to a common set: `pre_transit, label_created, in_transit, out_for_delivery, delivered, exception, returned, unknown`. Each ShipmentEvent keeps the carrier's own description, location, and timestamp plus when it was polled.

### 12.3 Carrier integration (the one unavoidable external dependency)

Package location lives at the carrier, so tracking is the one place pure self-host does not apply: the status must come from the carrier or an aggregator. Two paths:
- **Direct carrier APIs** (USPS, UPS, FedEx, DHL): no middleman, free within limits, but one integration per carrier and per-carrier auth. Best when the shop receives from a small fixed set of carriers.
- **A tracking aggregator** (EasyPost, AfterShip, Ship24, 17track-class): one integration, many carriers, at the cost of a third-party dependency and a fee. Best when carrier sprawl is annoying.

Default lean given the data-sovereignty line: direct carrier APIs for the carriers the shop actually receives from, aggregator as the fallback if that set grows (INV-OQ-30). Scraping carrier web pages is fragile and usually against terms; not recommended. Carrier credentials are stored server-side and are the single external egress in an otherwise self-hosted system.

### 12.4 Logging into the part's life

Shipment status happens before the parts are received, so units do not exist yet. Shipment events are logged on the Shipment (ShipmentEvent), attached to the Purchase. When the package is delivered and the order is received (units intaked), each resulting unit links to that Purchase, so the unit's life view (Section 16) shows the upstream shipment timeline followed by the unit's own events. The product's life is therefore the shipment history (pre-arrival, on the purchase) plus the unit events (post-intake), presented as one timeline.

### 12.5 Receiving reconciliation

A shipment the carrier marks `delivered` but that has not been received in the system is a "to receive" worklist item: the delivered signal is the prompt to intake the order and create units. The mismatch either way (tracking says delivered with no intake, or an intake with no matching delivered shipment) surfaces as a discrepancy.

---

## 13. Serial, barcode, and labels (PROPOSED)

Serials come from one of two places: the receipt, when the vendor prints them (Micro Center), or the physical part, scanned in-browser so any phone in the shop works. The scan is the default source; the receipt path pre-populates units and turns the scan into a verification step (13.4).

### 13.1 Scanning

Progressive enhancement, verified against 2026 browser support:

- **Native** `BarcodeDetector` where present (Chrome, Edge, Chrome on Android). Taps the OS vision framework, fast, no library weight.
- **WASM fallback** where absent (Safari, iOS, Firefox): zxing-wasm or ZBar-compiled-to-WASM (the `web-wasm-barcode-reader` package wraps ZBar via Emscripten and gets near-native speed; jsQR is a lighter QR-only option). A common `detect()`-style wrapper hides the difference.
- Requires a **secure context (HTTPS or localhost)**. Rear camera via `getUserMedia({ video: { facingMode: "environment" } })`.

Device implication: Android handsets use the native path only, no fallback to ship; iPhones in the mix make the WASM fallback mandatory (INV-OQ-9).

Formats to enable: `code_128, code_39, qr_code, data_matrix, ean_13, upc_a`. Covers most part serials (Code 128 and DataMatrix dominate), product UPCs, and the internal asset tags (13.5).

### 13.2 Serials with no barcode

Many parts print the serial as text only. Two fallbacks: photograph the sticker and OCR it through the extraction service (Section 11), operator confirms; or manual type.

### 13.3 Serial-format validation

On verification and manual entry, validate the captured serial against `Product.serial_format_regex` to catch the OCR confusion class (O vs 0, I vs 1, S vs 5). Warn, do not hard-block, since formats vary and exceptions exist.

### 13.4 Unit creation and the verification pass

A successful serial bind creates an InventoryUnit on the chosen line item, decrementing that line's units-remaining counter. Bulk items increment StockItem quantity instead.

When extraction returns serials on a line, units are created up front, pre-bound, `serial_source = receipt`, `verified = false`. The capture step then verifies:
- **Match** -> `verified = true`.
- **Mismatch** -> flag for the operator (wrong box, substitution, OCR slip). Resolve by correcting or re-scanning.
- **Partial** (quantity 4, two serials printed) -> two pre-bound units pending verification, the rest fall back to scan-as-source.

Worth scanning even when the receipt has the serial: it is the strongest RMA binding (exact serial tied to exact purchase line) plus physical confirmation the unit on the shelf is that unit, and it catches OCR and substitution before they become a bad RMA. Mandatory scan vs skip-with-flag is a policy call (INV-OQ-13). Default: pre-populate, mark `verified = false`, require the confirming scan for high-value categories (GPU, PSU, board, CPU), allow skip-with-flag for the rest.

The same scan-and-reconcile primitive backs the parts sweep on transfer (Section 6.5): there it compares the scanned set against the recorded system membership rather than a single receipt line.

### 13.5 Internal asset-tag printing

On intake each unit is assigned an internal ID and an asset tag is printed (QR or Code128 encoding the ID) and stuck on the unit and the bin. This closes the loop for parts whose OEM serial is unscannable or rubs off, gives bulk stock a scannable bin label, and makes every later re-scan fast since it reads the internal tag, not the OEM mark. Systems get their own tag too (Section 6.1). Hardware: a thermal label printer (ZPL for Zebra-class) or generated PDF label sheets for a standard printer (INV-OQ-19). `asset_tag` lives on InventoryUnit, StockItem, and System.

---

## 14. Landed-cost allocation (PROPOSED)

`unit_cost` is the landed cost, not the sticker line price: unit price plus the unit's share of order-level shipping and tax, net of line and order-level discounts. Newegg and Amazon receipts carry order-level shipping and coupons that have to spread across lines, so the allocation rule is explicit. Default: weight order-level shipping, tax, and discount by line total across all lines, then divide the line's landed total by its quantity to get per-unit cost. Alternatives are weight-by-quantity or weight-by-weight (INV-OQ-20). This makes resale margin and RMA valuation real numbers rather than the raw line price.

---

## 15. Kits and bundles (PROPOSED)

Two shapes break "one line equals N identical units":

- **RAM kit.** One Product (kit SKU), one box serial in the common case. Default: one unit by its box serial, optional `component_serials[]`. Parent/child component units only where sticks are tracked and RMA'd individually, per category (INV-OQ-18).
- **Combo deal.** One receipt line, combined price, different products (CPU + board). The line is `is_bundle = true` and expands at identity resolution into child PurchaseLineItems, one per component Product, each with an allocated price. Allocation default: weight by component MSRP; alternative is even split (INV-OQ-17). Each child spawns its own units, and landed cost (Section 14) flows through the allocated component price.

---

## 16. Unit event log and product life (PROPOSED)

`created_by` and `intake_by` record creation; the UnitEvent table records every mutation: status transitions, serial edits, verifications, reservations, installs, deliveries, ownership changes, re-validations, transfers, shipments, location changes, warranty registration, and every RMA action, each with actor, timestamp, and reason. Append-only.

The product's life is one timeline assembled from two sources: the upstream **shipment history** (ShipmentEvents on the unit's Purchase, which exist before the unit does, Section 12.4) followed by the unit's own **UnitEvents** (post-intake). A part's full story is therefore order placed, label created, in transit, delivered, received, intaked, built into system, delivered to customer, RMA opened, replaced, and so on, in order.

Two reasons it is load-bearing: it is the integrity backbone for RMA and transfer disputes (who changed what, when, why), and it is what the platform's OQ-40 provenance actually wants, the unit's history rather than just its current state. Logging the events from Phase 0 is cheap and makes this system the real answer to OQ-40. Wire event writes as each feature lands.

---

## 17. Receipt-to-part attachment

Receipt files land in object storage (Section 18). Each Purchase holds `receipt_files[]` (the stitched artifact plus original segments, Section 10.5). Because units chain to the purchase, the receipt is reachable from any unit for RMA, and the proof-of-purchase package (Section 7.4) is generated from it. Units additionally carry `photos[]`. For traded parts the proof image lives on the TradeIn record; for customer-owned parts the customer's proof is referenced on the unit.

---

## 18. Tech stack (PROPOSED, tradeoffs flagged, not pre-decided)

Aligned to your standing defaults: Rust core, Python isolated to ML, Postgres, self-host, no cloud (the one exception is carrier tracking, Section 12.3).

**Backend.** Rust + Axum (Tokio). Data access: SQLx (compile-time-checked queries against a live schema, fits your verification posture) or SeaORM (full ORM, more machinery). SQLx is the leaner default (INV-OQ-1).

**Database.** PostgreSQL 16/17. Relational fits inventory cleanly. JSONB holds `raw_extract`, event detail, validation snapshots, and carrier payloads. Built-in full-text search covers lookup now; add Meilisearch or Typesense only if search gets heavy.

**Object storage.** MinIO (self-hosted, S3-compatible, clean backup and replication) or plain filesystem behind the API (simplest for one box). For one shop, filesystem is enough; MinIO buys S3 semantics and easier backup discipline (INV-OQ-2).

**Extraction and stitching service.** Python (FastAPI) on the inference box, Section 11.3: templates, VLM, and OpenCV (rectification, screenshot row-match, ORB feature stitch, multi-page-PDF fallback).

**Shipment polling worker.** A scheduled Tokio task in the Rust backend (or a small separate worker binary), with per-carrier API clients or one aggregator client, rate-limited, persisting ShipmentEvents and stopping on delivery (Section 12).

**Label printing.** Thermal label printer driven by ZPL (Zebra-class) for the asset tags, or a PDF label-sheet generator for a standard printer (INV-OQ-19).

**Frontend.** The camera and barcode requirement, plus the guided multi-capture for long receipts, pulls toward browser-API ergonomics. Three honest paths (INV-OQ-3):
1. **Axum server-render (askama or maud) + HTMX + small JS islands** for the camera, capture, and sweep widgets. Least total code, server-driven. A little vanilla JS only for camera, the BarcodeDetector/WASM wrapper, and the overlapping-capture overlay.
2. **Leptos (or Dioxus) all-Rust WASM.** One language. Cost: `getUserMedia`, BarcodeDetector / zxing-wasm, and the capture overlay through web-sys is more verbose.
3. **TS SPA (SolidJS or Svelte)** against the Axum JSON API. Cleanest browser-API access and the most scanner libraries, at the cost of a second language and toolchain.

The camera and scan widgets (intake, verification, long-receipt capture, parts sweep) are the only place the choice bites; the rest is forms and tables.

**Auth and exposure.** Single-tenant, a few operator accounts. Put the app behind your existing Headscale mesh so it is not internet-exposed, with a thin app login (argon2, session cookies) on top. The polling worker's outbound carrier calls are the only traffic that leaves the mesh.

**Deployment.** Single box, Docker Compose: `postgres`, `minio` (optional), `inventory-api` (Rust), `poller` (Rust, or a task inside the API). The Python extractor/stitcher runs on the inference box over HTTP. No cloud dependency beyond the carrier tracking endpoints.

**Backup, DR, and export.** Scheduled `pg_dump` plus optional WAL archiving for point-in-time recovery, and object-store backup (MinIO replication or filesystem rsync). Plus a plain CSV/JSON export of the full inventory (units, purchases, shipments, systems, validations, events) for portability. Your data-sovereignty line wants a no-lock-in flat export regardless of where the data lives.

---

## 19. cec.direct seam (design now, build later)

Do not couple. Define the surface so the later integration is clean.

The inventory system owns unit and system state and is the natural home for the build-to-serial record. The seam is two operations cec.direct calls:
- **Availability read:** units `in_stock` by product, plus bulk quantity-on-hand.
- **Reserve / consume:** transition a unit `in_stock -> reserved -> in_build -> installed` (and decrement bulk) as a build pulls parts, and attach the unit to a System whose `build_id` references the cec.direct build.

The "which serial went into which machine" record is the System membership plus unit serials, kept accurate across RMA by the re-validation flow and the replacement inheriting `system_id` (Section 7.7), with a `provenance_stale` flag when a customer-run RMA hides a new serial. This is the missing input the platform spec already calls for: Concierge's outcome-label ingestion (platform OQ-40) wants RMA, failure, and service events tied to a unit ID, and identity-and-provenance (platform OQ-44) wants the unit-and-module inventory per build. This inventory system is a direct candidate answer to the "RMA-system integration" path named in OQ-40.

Mechanism: HTTP/JSON API if cec.direct is a separate service, or a shared Postgres schema if cec.direct is also Rust + Postgres (INV-OQ-10).

Note (deferred, your call): the inventory feeds two consumers, not one. The Shopify retail channel CHH Chris runs decrements stock the same as a build does, so the consume seam can be defined as two consumers when you choose to wire it. Left out of this revision per scope.

---

## 20. Build phases

Phase 0 stands alone and already satisfies "keep track of everything." Each phase is shippable.

- **Phase 0: schema + spine.** The full schema (Section 4), including `owner`, the System / CecWarrantyPolicy / SystemValidation / SystemTransfer tables, the Shipment / ShipmentEvent tables, the unit warranty and `system_id` fields, `purchase_datetime`, the expanded RmaCase, UnitEvent, kit/bundle line structure, asset-tag fields, and landed-cost fields, even where their UI ships later. Rust + Axum API skeleton, minimal web UI. Manual purchase, unit, and bulk entry, receipt file upload, event logging on every write. Usable day one.
- **Phase 1: receipt capture, extraction, cost, and order tracking.** Object storage wired. Python extraction service up (stitching + template fast-path + VLM fallback), whole-page web capture and guided overlapping capture for long receipts. Receipt to auto-populated line items, operator confirms. Identity resolution with bundle expansion. Landed-cost allocation. Shipment capture and the polling worker, with carrier status logged to the order.
- **Phase 2: capture + labels + migration.** Browser scan loop (native + WASM fallback), serial-to-unit binding and the verification pass, serial-format validation, asset-tag printing, opening-balance intake for existing stock.
- **Phase 3: ownership, delivery, and the two warranties.** Ownership-aware readiness, the delivery flow (shop -> customer) that starts the CEC warranty clock, the two-warranty display per part and per system, and the RMA lifecycle (three execution modes, proof-of-purchase package, replacement intake with remainder-of-term warranty, advance-replacement holds, vendor-return-vs-manufacturer decision, custody tracking).
- **Phase 4: systems, re-validation, and transfer.** System validation state, the re-validation flow that restores CEC coverage after a change, the parts sweep, and the warranty-transfer path (customer -> customer) gated on a clean sweep. Provenance update on replacement.
- **Phase 5: cec.direct seam.** Availability and reserve/consume API, System `build_id` linkage, field-RMA provenance recovery.

Cross-cutting, slot anytime: reorder workflow behind `reorder_point` (alert or suggested-restock list), receiving reconciliation against delivered shipments (Section 12.5), and the backup/DR plus CSV/JSON export.

---

## 21. Open questions

- **INV-OQ-1:** Data access layer, SQLx vs SeaORM.
- **INV-OQ-2:** Object storage, MinIO vs filesystem.
- **INV-OQ-3:** Frontend, Axum+HTMX+islands vs Leptos vs TS SPA.
- **INV-OQ-4:** Extraction model choice (Qwen2.5-VL-7B vs dots.ocr/DeepSeek-OCR vs PaddleOCR+text-LLM) and host (Jetson Orin vs K3).
- **INV-OQ-5:** Template-parser coverage order. Which vendors get deterministic parsers first; vendors that print serials in a stable format rank higher.
- **INV-OQ-6:** UPC enrichment. Public UPC/EAN databases as identity hints vs skip (coverage for PC components and MPNs is unreliable; the authoritative catalog is internal).
- **INV-OQ-7:** Serialized-vs-bulk policy per category. Confirm the serialize list (GPU, PSU, board, CPU, RAM, storage, CEC modules) and the bulk list (cables, screws, paste, passives).
- **INV-OQ-8:** Bulk cost basis, moving-average vs FIFO lots. Serialized units carry exact per-unit landed cost; only bulk needs this.
- **INV-OQ-9:** Shop scanning handset. Android (native scan only) vs iPhone in the mix (WASM fallback mandatory).
- **INV-OQ-10:** cec.direct seam, shared Postgres vs HTTP API.
- **INV-OQ-11:** Warranty term source, manufacturer default per Product vs per-unit override, and where registration extensions get recorded.
- **INV-OQ-12:** Location/bin granularity. Flat bins now vs structured locations later.
- **INV-OQ-13:** Receipt-serial verification policy. Mandatory confirming scan for high-value categories vs receipt serial standing alone with a flag.
- **INV-OQ-14:** RMA execution-mode default. When to ask the customer to ship back (mode 2) vs run it themselves with assistance (mode 3) vs CEC-managed (mode 1), by ownership and fault type.
- **INV-OQ-15:** Replacement warranty basis per manufacturer (original remainder vs a replacement term, commonly 90 days), and the default when unknown.
- **INV-OQ-16:** Vendor return windows. Populate `return_window_days` and `doa_window_days` per vendor and category.
- **INV-OQ-17:** Bundle price allocation rule. MSRP-weighted vs even split across combo components.
- **INV-OQ-18:** Kit serial model. Kit-as-single-unit by box serial (default) vs parent/child component units, per category.
- **INV-OQ-19:** Asset-tag hardware. Thermal/ZPL printer vs PDF label sheets, and symbology (QR vs Code128).
- **INV-OQ-20:** Landed-cost allocation rule. Weight order shipping/tax/discount by line total (default) vs by quantity vs by weight.
- **INV-OQ-21:** Field-RMA provenance recovery. How CEC learns the replacement serial when the customer runs the RMA (customer reports back, re-scan on next service, or accept the stale flag).
- **INV-OQ-22:** Manufacturer warranty start basis, invoice/purchase datetime vs manufacture date, per manufacturer, and whether sub-day time granularity is ever load-bearing or stored for completeness only.
- **INV-OQ-23:** CEC-provided warranty terms. Full-class and refurb-class term lengths (and any per-category split), as `CecWarrantyPolicy` values.
- **INV-OQ-24:** CEC warranty clock during invalidation. Runs continuously with coverage suspended (default) vs pauses until re-validated (`clock_pauses_when_invalidated`).
- **INV-OQ-25:** Re-validation method and pass criteria. CEC bench check vs the platform EOL gate / Concierge golden comparison, and the minimum pass set after a change.
- **INV-OQ-26:** Manufacturer transferability. Which manufacturers void warranty on resale or for a non-original purchaser, captured as `Manufacturer.warranty_transferable`.
- **INV-OQ-27:** CEC warranty on transfer. Carried vs reset vs pro-rated, transfer fee, and whether a fresh re-validation is required when the last one is stale.
- **INV-OQ-28:** Parts-sweep discrepancy handling. What a `serial_mismatch` / `missing` / `unexpected_extra` does to a transfer (hard block, per-part warranty void, or bring-to-record then re-validate).
- **INV-OQ-29:** System identity. Build number vs a CEC-assigned system serial vs an asset tag, and the system's own scannable tag.
- **INV-OQ-30:** Carrier integration. Direct carrier APIs (USPS/UPS/FedEx/DHL) vs a tracking aggregator (EasyPost/AfterShip/Ship24), and which carriers the shop actually receives from.
- **INV-OQ-31:** Poll cadence and retention. How often to poll active shipments, how much to tighten near delivery, and how long to retain ShipmentEvent history.
- **INV-OQ-32:** Web-receipt capture default. Whole-page screenshot vs print-to-PDF vs a browser-extension capture, and the trigger for the screenshot-stitch fallback.
- **INV-OQ-33:** Stitch failure threshold. The confidence below which the system stops trying to pixel-stitch photos and falls back to the multi-page-PDF plus VLM-ordered-pages path.

---

## 22. What you would need, in one list

- Postgres box (the schema in Section 4).
- Rust + Axum service (the API and, on path 1 or 2, the UI) plus the shipment polling worker.
- Object store (MinIO or a filesystem dir).
- Python extraction + stitching service on the inference box (OpenCV, templates, one VLM).
- A phone or two for capture, long-receipt capture, and parts sweeps, on the shop network behind Headscale (handset choice gates the scanner fallback).
- A label printer (thermal/ZPL or a standard printer for PDF sheets) for unit, bin, and system tags.
- Carrier tracking access: direct carrier API credentials, or one aggregator account.
- Per-vendor receipt templates and per-vendor return-window data for your recurring suppliers.
- CEC-provided warranty terms (full and refurb) as policy values.
- An internal product catalog, seeded as you ingest the first receipts and existing stock.

Phase 0 needs only the first three. Everything past that is additive.

---

## Revision history

- **0.4.0** (2026-06-26): Added multi-image receipt capture and automatic stitching (whole-page capture preferred for web, guided overlapping capture plus OpenCV stitching for paper, degrade-to-multipage-PDF on stitch failure) and order tracking with carrier polling (Shipment and ShipmentEvent entities, a polling worker, normalized status, the direct-carrier-vs-aggregator decision as the single external dependency, and shipment history folded into the part's life timeline). Extraction schema gained `shipments`. New Sections 10 (capture/stitching) and 12 (tracking); the unit event log became Section 16 "event log and product life." Added INV-OQ-30 through INV-OQ-33. Fixed stale cross-references in Section 5 that pointed at a nonexistent 6.9 (the replacement and registration subsections are 7.6 and 7.9).
- **0.3.0** (2026-06-26): Added the System entity, the two-layer warranty model (manufacturer from purchase datetime, CEC-provided from delivery, classed full or refurb via CecWarrantyPolicy), exact `purchase_datetime`, the delivery flow, the re-validation flow gating CEC coverage, and the warranty-transfer path gated on a documented parts sweep. New entities System, CecWarrantyPolicy, SystemValidation, SystemTransfer; manufacturer warranty fields renamed `mfr_warranty_*`. Added INV-OQ-22 through INV-OQ-29.
- **0.2.0** (2026-06-26): RMA lifecycle and three execution modes, ownership (shop vs customer), the proof-of-purchase package, vendor-return-vs-manufacturer, replacement intake with remainder-of-term warranty and advance-replacement holds, the append-only unit event log, landed-cost allocation, kits and bundles, internal asset-tag printing, opening-balance intake, serial-format validation, warranty registration, reorder workflow, and backup/DR plus CSV/JSON export. New entities VendorReturnPolicy and UnitEvent. Added INV-OQ-14 through INV-OQ-21. Shopify second-consumer and overseas import/FX noted but deferred.
- **0.1.1** (2026-06-26): Corrected the absolute "receipts have no serials" claim; serials are vendor-dependent and Micro Center prints them. Added the receipt-carried-serial path and the verification pass, `serial_source` and `verified` fields, and INV-OQ-13.
- **0.1.0** (2026-06-26): Initial scope.
