-- Create the classification_source_type enum and classification_sources table.
-- Every classification row (in domain_classifications) references one source,
-- indicating whether it came from an LLM prompt, a human admin, or an exclude
-- rule.  This replaces the ad-hoc prompt_id FK with a proper discriminated
-- union that accommodates all current and anticipated source types.
CREATE TYPE classification_source_type AS ENUM (
  'llm_prompt',          -- LLM classified the domain using a prompt.
  'admin',               -- A human admin made the decision via the UI.
  'config_exclude_rule', -- Matched a pattern declared in config / startup args.
  'manual_exclude_rule'  -- Matched a pattern added at runtime via the UI.
);

CREATE TABLE classification_sources (
  id          SERIAL PRIMARY KEY,
  source_type classification_source_type NOT NULL,

  -- Set only for 'llm_prompt' sources; references the exact prompt text used.
  prompt_id   INTEGER REFERENCES prompts(id),

  -- Set only for 'admin' sources; identifies the user who made the decision.
  user_id     INTEGER REFERENCES users(id),

  -- Set only for exclude-rule sources; the pattern that was matched.
  -- Serves as the deduplication key for exclude rules.
  label       TEXT,

  created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

  CONSTRAINT llm_prompt_requires_prompt_id
    CHECK (source_type != 'llm_prompt' OR prompt_id IS NOT NULL),
  CONSTRAINT admin_requires_user
    CHECK (source_type != 'admin' OR user_id IS NOT NULL)
);

-- One source per unique prompt (dedup by prompt_id).
CREATE UNIQUE INDEX idx_classification_sources_prompt_id
  ON classification_sources(prompt_id)
  WHERE prompt_id IS NOT NULL;

-- One source per (type, pattern) for exclude rules (dedup by label).
CREATE UNIQUE INDEX idx_classification_sources_exclude_label
  ON classification_sources(source_type, label)
  WHERE label IS NOT NULL;

-- Backfill: create an 'llm_prompt' source for every existing prompt so that
-- the migration of prompt_id → source_id can be done in one pass.
INSERT INTO classification_sources (source_type, prompt_id, created_at)
SELECT 'llm_prompt', id, created_at
FROM prompts;
