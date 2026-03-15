-- Enforce the singleton constraint for the 'provisioned' source row.
-- This must be a separate migration from the ALTER TYPE ADD VALUE above because
-- PostgreSQL requires newly added enum values to be committed before they can
-- appear in a partial index WHERE clause.
CREATE UNIQUE INDEX idx_classification_sources_provisioned_singleton
  ON classification_sources (source_type)
  WHERE source_type = 'provisioned';
