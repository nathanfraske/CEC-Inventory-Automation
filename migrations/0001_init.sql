-- CEC Inventory Phase 0 schema (working basis). Mirrors scope Section 4.
-- Postgres 16. UUID PKs via gen_random_uuid() (core, no extension). timestamptz throughout.
-- Computed warranty/eligibility columns are app-set plain columns (logic is in the app,
-- not a simple SQL expression), see scope Sections 5 and 6.
-- Add values to any enum later with: ALTER TYPE <name> ADD VALUE '<x>'; (non-blocking).

-- ---------- enums ----------
CREATE TYPE owner_kind          AS ENUM ('shop','customer');
CREATE TYPE source_type         AS ENUM ('physical_photo','pdf','email','manual','trade_in','opening_balance');
CREATE TYPE resolution_status   AS ENUM ('unresolved','suggested','confirmed');
CREATE TYPE serial_source       AS ENUM ('receipt','scan','ocr','manual');
CREATE TYPE condition_kind      AS ENUM ('new','open_box','used','refurb','unknown');
CREATE TYPE acquisition_method  AS ENUM ('purchase','trade_in','rma_replacement','gift','salvage','opening_balance');
CREATE TYPE unit_status         AS ENUM ('in_stock','reserved','in_build','installed','with_customer','shipped','rma_open','pending_return','defective','returned','scrapped');
CREATE TYPE mfr_warranty_basis  AS ENUM ('standard','original_remainder','replacement_term','registered');
CREATE TYPE cec_warranty_class  AS ENUM ('full','refurb','none');
CREATE TYPE warranty_start_basis AS ENUM ('purchase_datetime','manufacture_date');
CREATE TYPE warranty_basis_default AS ENUM ('original_remainder','replacement_term');
CREATE TYPE cost_basis_kind     AS ENUM ('moving_avg','per_lot');
CREATE TYPE system_status       AS ENUM ('in_build','in_stock','delivered','in_service','rma_open','retired');
CREATE TYPE validation_state    AS ENUM ('validated','invalidated','pending_revalidation');
CREATE TYPE validation_type     AS ENUM ('eol','post_change','periodic','pre_transfer','sweep');
CREATE TYPE validation_trigger  AS ENUM ('build_complete','rma','parts_swap','service','transfer_request','audit');
CREATE TYPE validation_result   AS ENUM ('pass','fail');
CREATE TYPE cec_warranty_outcome AS ENUM ('carried','reset','prorated','declined');
CREATE TYPE transfer_result     AS ENUM ('completed','blocked_on_sweep','partial');
CREATE TYPE carrier_kind        AS ENUM ('usps','ups','fedex','dhl','other');
CREATE TYPE shipment_status     AS ENUM ('pre_transit','label_created','in_transit','out_for_delivery','delivered','exception','returned','unknown');
CREATE TYPE poll_state          AS ENUM ('active','stopped');
CREATE TYPE proof_status        AS ENUM ('provided','customer_has_will_send','customer_lacks','none');
CREATE TYPE rma_party           AS ENUM ('vendor','manufacturer');
CREATE TYPE rma_execution_mode  AS ENUM ('cec_managed','customer_ships_to_cec','customer_managed_assist');
CREATE TYPE rma_proof_source    AS ENUM ('cec_receipt','customer_receipt');
CREATE TYPE rma_custody         AS ENUM ('at_cec','with_customer','in_transit_to_cec','in_transit_to_vendor','in_transit_to_customer');
CREATE TYPE rma_status          AS ENUM ('open','info_provided_to_customer','awaiting_customer_action','shipped_to_vendor','awaiting_replacement','replacement_received','replacement_with_customer','refunded','denied','closed');
CREATE TYPE unit_event_type     AS ENUM ('intake','status_change','serial_edit','verify','reserve','install','deliver','ship','location_change','owner_change','warranty_registered','revalidated','transfer','rma_open','rma_update','replace_out','replace_in','scrap','note');

-- ---------- catalog and vendors ----------
CREATE TABLE vendor (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name          text NOT NULL,
  address       text,
  website       text,
  rma_url       text,
  rma_contact   text,
  account_number text,
  notes         text
);

CREATE TABLE vendor_return_policy (
  id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  vendor_id         uuid NOT NULL REFERENCES vendor(id) ON DELETE CASCADE,
  category          text,                 -- NULL = default for the vendor
  return_window_days int,
  doa_window_days   int,
  restocking_fee_pct numeric(5,2),
  notes             text
);

CREATE TABLE manufacturer (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name          text NOT NULL,
  rma_url       text,
  rma_contact   text,
  warranty_policy_url text,
  default_warranty_months int,
  replacement_warranty_days int DEFAULT 90,
  warranty_basis_default warranty_basis_default DEFAULT 'replacement_term',
  warranty_transferable boolean DEFAULT true,
  warranty_start_basis warranty_start_basis DEFAULT 'purchase_datetime',
  notes         text
);

CREATE TABLE product (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  manufacturer_id uuid REFERENCES manufacturer(id),
  model           text NOT NULL,
  mpn             text,
  upc_ean         text,
  category        text,
  serialized      boolean NOT NULL DEFAULT true,
  default_warranty_months int,            -- override of manufacturer default
  serial_format_regex text,
  datasheet_url   text,
  notes           text
);
CREATE INDEX product_mpn_idx ON product (mpn);
CREATE INDEX product_upc_idx ON product (upc_ean);

-- ---------- systems and CEC warranty policy ----------
CREATE TABLE cec_warranty_policy (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  warranty_class cec_warranty_class NOT NULL,
  category      text,                      -- NULL = default
  term_months   int NOT NULL,
  transferable  boolean NOT NULL DEFAULT true,
  reset_on_transfer boolean NOT NULL DEFAULT false,
  clock_pauses_when_invalidated boolean NOT NULL DEFAULT false,
  notes         text
);

CREATE TABLE system (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  label           text,                    -- build number / system serial
  asset_tag       text UNIQUE,
  build_id        uuid,                    -- ref to cec.direct build (seam, scope Section 19)
  current_owner   owner_kind NOT NULL DEFAULT 'shop',
  customer_ref    text,
  status          system_status NOT NULL DEFAULT 'in_build',
  delivery_datetime timestamptz,
  cec_warranty_class cec_warranty_class,
  validation_state validation_state NOT NULL DEFAULT 'pending_revalidation',
  provenance_stale boolean NOT NULL DEFAULT false,
  last_validated_at timestamptz,
  last_validated_by text,
  notes           text
);

-- ---------- purchases, shipments, line items ----------
CREATE TABLE purchase (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  vendor_id       uuid REFERENCES vendor(id),     -- NULL for opening_balance unknown origin
  purchase_datetime timestamptz,
  order_number    text,
  invoice_number  text,
  currency        text DEFAULT 'USD',
  subtotal        numeric(12,2),
  tax             numeric(12,2),
  shipping        numeric(12,2),
  discount_total  numeric(12,2),
  total           numeric(12,2),
  payment_method  text,
  source_type     source_type NOT NULL,
  receipt_files   jsonb DEFAULT '[]'::jsonb,       -- object-store refs (stitched + segments)
  raw_extract     jsonb,
  extract_confidence numeric(4,3),
  created_by      text,
  created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE purchase_line_item (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  purchase_id     uuid NOT NULL REFERENCES purchase(id) ON DELETE CASCADE,
  product_id      uuid REFERENCES product(id),     -- NULL until resolved
  description_as_printed text,
  vendor_sku      text,
  quantity        int NOT NULL DEFAULT 1,
  unit_price      numeric(12,2),
  line_total      numeric(12,2),
  currency        text DEFAULT 'USD',
  is_bundle       boolean NOT NULL DEFAULT false,
  parent_line_id  uuid REFERENCES purchase_line_item(id),
  allocated_landed_cost numeric(12,2),
  resolution_status resolution_status NOT NULL DEFAULT 'unresolved'
);
CREATE INDEX pli_purchase_idx ON purchase_line_item (purchase_id);

CREATE TABLE shipment (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  purchase_id     uuid NOT NULL REFERENCES purchase(id) ON DELETE CASCADE,
  carrier         carrier_kind,
  tracking_number text,
  tracking_url    text,
  status          shipment_status NOT NULL DEFAULT 'unknown',
  expected_delivery_date date,
  shipped_at      timestamptz,
  delivered_at    timestamptz,
  last_polled_at  timestamptz,
  poll_state      poll_state NOT NULL DEFAULT 'active',
  line_item_ids   uuid[],                  -- split-shipment subset (optional)
  notes           text
);
CREATE INDEX shipment_active_idx ON shipment (poll_state) WHERE poll_state = 'active';

CREATE TABLE shipment_event (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  shipment_id     uuid NOT NULL REFERENCES shipment(id) ON DELETE CASCADE,
  event_status    shipment_status NOT NULL,
  carrier_description text,
  location        text,
  occurred_at     timestamptz,             -- carrier timestamp
  polled_at       timestamptz NOT NULL DEFAULT now(),
  raw             jsonb
);
CREATE INDEX shipment_event_ship_idx ON shipment_event (shipment_id, occurred_at);

-- ---------- inventory units and bulk stock ----------
CREATE TABLE inventory_unit (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  product_id      uuid REFERENCES product(id),
  line_item_id    uuid REFERENCES purchase_line_item(id),  -- NULL for trade-in/opening/unknown
  system_id       uuid REFERENCES system(id),              -- build linkage via system
  owner           owner_kind NOT NULL DEFAULT 'shop',
  customer_ref    text,
  serial_number   text,
  serial_source   serial_source,
  verified        boolean NOT NULL DEFAULT false,
  asset_tag       text UNIQUE,
  condition       condition_kind NOT NULL DEFAULT 'new',
  acquisition_method acquisition_method NOT NULL DEFAULT 'purchase',
  status          unit_status NOT NULL DEFAULT 'in_stock',
  location_bin    text,
  unit_cost       numeric(12,2),           -- landed
  mfr_warranty_expires date,               -- app-computed
  mfr_warranty_basis mfr_warranty_basis DEFAULT 'standard',
  cec_warranty_class cec_warranty_class DEFAULT 'none',
  cec_warranty_start timestamptz,
  cec_warranty_expires date,               -- app-computed
  replaces_unit_id uuid REFERENCES inventory_unit(id),
  registered      boolean NOT NULL DEFAULT false,
  rma_eligible    boolean,                 -- app-computed, overridable
  rma_block_reason text,
  photos          jsonb DEFAULT '[]'::jsonb,
  notes           text,
  intake_at       timestamptz NOT NULL DEFAULT now(),
  intake_by       text
);
CREATE INDEX unit_serial_idx ON inventory_unit (serial_number);
CREATE INDEX unit_system_idx ON inventory_unit (system_id);
CREATE INDEX unit_status_idx ON inventory_unit (status);

CREATE TABLE stock_item (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  product_id      uuid NOT NULL REFERENCES product(id),
  location_bin    text,
  asset_tag       text,
  quantity_on_hand int NOT NULL DEFAULT 0,
  reorder_point   int,
  cost_basis      cost_basis_kind NOT NULL DEFAULT 'moving_avg',
  notes           text
);

-- ---------- system validation and transfer ----------
CREATE TABLE system_validation (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  system_id       uuid NOT NULL REFERENCES system(id) ON DELETE CASCADE,
  validation_type validation_type NOT NULL,
  trigger         validation_trigger,
  performed_at    timestamptz NOT NULL DEFAULT now(),
  performed_by    text,
  result          validation_result,
  parts_snapshot  jsonb,                   -- unit_id + serial at validation time
  reconciliation  jsonb,                   -- per-unit match results when a sweep
  evidence_refs   jsonb DEFAULT '[]'::jsonb,
  notes           text
);
CREATE INDEX sysval_system_idx ON system_validation (system_id, performed_at);

CREATE TABLE system_transfer (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  system_id       uuid NOT NULL REFERENCES system(id) ON DELETE CASCADE,
  from_owner_ref  text,
  to_owner_ref    text,
  transfer_datetime timestamptz NOT NULL DEFAULT now(),
  performed_by    text,
  sweep_id        uuid REFERENCES system_validation(id),
  mfr_warranty_outcome jsonb,              -- per-part carried | void_non_transferable
  cec_warranty_outcome cec_warranty_outcome,
  cec_transfer_fee numeric(12,2),
  result          transfer_result,
  notes           text
);

-- ---------- trade-ins ----------
CREATE TABLE trade_in (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  customer_ref    text,
  trade_date      timestamptz NOT NULL DEFAULT now(),
  source_notes    text,
  proof_of_purchase_status proof_status NOT NULL DEFAULT 'none',
  proof_files     jsonb DEFAULT '[]'::jsonb
);

CREATE TABLE trade_in_unit (
  trade_in_id     uuid NOT NULL REFERENCES trade_in(id) ON DELETE CASCADE,
  unit_id         uuid NOT NULL REFERENCES inventory_unit(id) ON DELETE CASCADE,
  PRIMARY KEY (trade_in_id, unit_id)
);

-- ---------- RMA ----------
CREATE TABLE rma_case (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  unit_id         uuid NOT NULL REFERENCES inventory_unit(id),
  owner_at_failure owner_kind,
  party           rma_party,
  execution_mode  rma_execution_mode,
  proof_source    rma_proof_source,
  custody         rma_custody,
  rma_number      text,
  fault_description text,
  status          rma_status NOT NULL DEFAULT 'open',
  assist_artifacts jsonb,                  -- proof_package_ref, portal_link, contact, sent_at
  advance_replacement boolean NOT NULL DEFAULT false,
  auth_hold_ref   text,
  return_due_date date,
  opened_at       timestamptz NOT NULL DEFAULT now(),
  closed_at       timestamptz,
  shipped_at      timestamptz,
  return_tracking text,
  replacement_unit_id uuid REFERENCES inventory_unit(id),
  resolution      text,
  notes           text
);
CREATE INDEX rma_unit_idx ON rma_case (unit_id);

-- ---------- append-only unit event log ----------
CREATE TABLE unit_event (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  unit_id         uuid NOT NULL REFERENCES inventory_unit(id) ON DELETE CASCADE,
  event_type      unit_event_type NOT NULL,
  from_value      text,
  to_value        text,
  actor           text,
  at              timestamptz NOT NULL DEFAULT now(),
  system_id       uuid REFERENCES system(id),
  rma_case_id     uuid REFERENCES rma_case(id),
  detail          jsonb
);
CREATE INDEX unit_event_unit_idx ON unit_event (unit_id, at);
