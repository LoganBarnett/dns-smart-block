-- Add 'provisioned' to classification_source_type.
-- Provisioned classifications are declared in NixOS configuration and managed
-- exclusively by `dns-smart-block-cli domain reconcile`.  Reconcile upserts
-- all desired entries and expires any provisioned rows that are no longer
-- declared — without touching 'admin'-sourced rows.
--
-- The partial unique index that enforces the singleton source row is in the
-- next migration (000005) because PostgreSQL requires a new enum value to be
-- committed before it can appear in a WHERE clause or expression.
ALTER TYPE classification_source_type ADD VALUE 'provisioned';
