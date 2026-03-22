-- Create provisioned_pattern_rules table.
-- Each row represents one active (or expired) regex-based classification rule.
-- Rules are managed by `dns-smart-block-cli domain reconcile` (for provisioned)
-- or via the admin classify endpoint (for admin-originated patterns).
--
-- Source dedup: each unique (pattern, classification_type) pair shares one
-- classification_sources row with source_type = 'provisioned_pattern' and
-- label = '<pattern>|<classification_type>'.  The existing
-- idx_classification_sources_exclude_label index (unique on (source_type, label)
-- WHERE label IS NOT NULL) enforces this deduplication.
--
-- Admin-originated pattern rules use source_type = 'admin' in
-- classification_sources and are never touched by reconcile.
CREATE TABLE provisioned_pattern_rules (
  id               SERIAL PRIMARY KEY,
  pattern          TEXT        NOT NULL,
  classification_type TEXT     NOT NULL,
  is_matching_site BOOLEAN     NOT NULL,
  confidence       REAL        NOT NULL DEFAULT 1.0,
  reasoning        TEXT,
  source_id        INTEGER REFERENCES classification_sources(id),
  valid_on         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  valid_until      TIMESTAMPTZ NOT NULL,
  created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for fetching currently-active rules efficiently.
CREATE INDEX idx_ppr_valid ON provisioned_pattern_rules (valid_on, valid_until);
