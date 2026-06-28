-- 0006: one return/warranty policy per (key, category), incl. a single default (NULL category).
-- Without these, vendor_return_policy / cec_warranty_policy can each hold multiple rows matching the
-- same (vendor|warranty_class, category), and a policy lookup silently returns an arbitrary one —
-- inconsistent warranty/return decisions in the RMA path (audit 2026-06-27 data-integrity backlog).
-- A plain UNIQUE(.., category) is insufficient: SQL treats NULL categories as DISTINCT, so two
-- "default" rows would slip through. Use partial unique indexes — one for explicit categories, one
-- for the single NULL-category default. Assumes existing rows are already unique (true on a clean DB).

-- vendor_return_policy: unique per (vendor, category); at most one default (category IS NULL).
CREATE UNIQUE INDEX vendor_return_policy_vendor_category_uniq
    ON vendor_return_policy (vendor_id, category)
    WHERE category IS NOT NULL;
CREATE UNIQUE INDEX vendor_return_policy_vendor_default_uniq
    ON vendor_return_policy (vendor_id)
    WHERE category IS NULL;

-- cec_warranty_policy: unique per (warranty_class, category); at most one default per class.
CREATE UNIQUE INDEX cec_warranty_policy_class_category_uniq
    ON cec_warranty_policy (warranty_class, category)
    WHERE category IS NOT NULL;
CREATE UNIQUE INDEX cec_warranty_policy_class_default_uniq
    ON cec_warranty_policy (warranty_class)
    WHERE category IS NULL;
