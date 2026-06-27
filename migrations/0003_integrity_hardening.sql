-- 0003: data-integrity hardening from the audit (docs/AUDIT-2026-06-27.md).
-- Append-only migration (never edit an applied one). On an existing box with duplicate serials
-- or asset tags, deduplicate before applying — the unique indexes below will otherwise fail.

-- 1) Serial numbers are GLOBALLY UNIQUE (owner decision 2026-06-27). Replace the plain index
--    with a partial unique index so many NULL serials are still allowed (units pre-serial).
DROP INDEX IF EXISTS unit_serial_idx;
CREATE UNIQUE INDEX unit_serial_unique_idx
    ON inventory_unit (serial_number)
    WHERE serial_number IS NOT NULL;

-- 2) Bulk-stock asset tags are unique too (inventory_unit/system already are), so a scanned
--    asset tag resolves to exactly one entity.
ALTER TABLE stock_item ADD CONSTRAINT stock_item_asset_tag_key UNIQUE (asset_tag);

-- 3) Enforce the append-only invariant on the audit/event tables at the DB level (it was a
--    convention only). Any UPDATE/DELETE — including a cascade that would erase history —
--    raises, so the integrity backbone (CLAUDE.md §3, scope §16) can't be rewritten.
CREATE OR REPLACE FUNCTION cec_append_only() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'table % is append-only; % is not permitted', TG_TABLE_NAME, TG_OP;
END;
$$;

CREATE TRIGGER unit_event_append_only
    BEFORE UPDATE OR DELETE ON unit_event
    FOR EACH ROW EXECUTE FUNCTION cec_append_only();

CREATE TRIGGER system_validation_append_only
    BEFORE UPDATE OR DELETE ON system_validation
    FOR EACH ROW EXECUTE FUNCTION cec_append_only();

CREATE TRIGGER system_transfer_append_only
    BEFORE UPDATE OR DELETE ON system_transfer
    FOR EACH ROW EXECUTE FUNCTION cec_append_only();

CREATE TRIGGER shipment_event_append_only
    BEFORE UPDATE OR DELETE ON shipment_event
    FOR EACH ROW EXECUTE FUNCTION cec_append_only();
