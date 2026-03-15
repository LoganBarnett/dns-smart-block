-- Replace prompt_id with source_id on domain_classifications and
-- domain_classification_events.  Both columns stay nullable so that
-- historical rows that predate source tracking remain valid.

-- ── domain_classifications ───────────────────────────────────────────────────

ALTER TABLE domain_classifications
  ADD COLUMN source_id INTEGER REFERENCES classification_sources(id);

-- Backfill: every classification that had a prompt now points to the
-- corresponding classification_source row created in the previous migration.
UPDATE domain_classifications dc
  SET source_id = cs.id
  FROM classification_sources cs
  WHERE dc.prompt_id = cs.prompt_id
    AND cs.source_type = 'llm_prompt';

DROP INDEX IF EXISTS idx_classifications_prompt;
ALTER TABLE domain_classifications DROP COLUMN prompt_id;

CREATE INDEX idx_classifications_source_id
  ON domain_classifications(source_id);

-- ── domain_classification_events ─────────────────────────────────────────────

ALTER TABLE domain_classification_events
  ADD COLUMN source_id INTEGER REFERENCES classification_sources(id);

-- Backfill: same logic as above.
UPDATE domain_classification_events dce
  SET source_id = cs.id
  FROM classification_sources cs
  WHERE dce.prompt_id = cs.prompt_id
    AND cs.source_type = 'llm_prompt';

DROP INDEX IF EXISTS idx_domain_classification_events_prompt_id;
ALTER TABLE domain_classification_events DROP COLUMN prompt_id;

CREATE INDEX idx_domain_classification_events_source_id
  ON domain_classification_events(source_id);
