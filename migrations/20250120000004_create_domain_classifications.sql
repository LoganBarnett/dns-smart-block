-- Create domain_classifications table - projection of classification results
CREATE TABLE IF NOT EXISTS domain_classifications (
    id SERIAL PRIMARY KEY,
    domain TEXT NOT NULL REFERENCES domains(domain) ON DELETE CASCADE,
    classification_type TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    valid_on TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    valid_until TIMESTAMPTZ NOT NULL DEFAULT (NOW() + INTERVAL '10 days'),
    model TEXT NOT NULL,
    prompt_id INTEGER NOT NULL REFERENCES prompts(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for domain lookups with valid classifications
CREATE INDEX IF NOT EXISTS idx_classifications_domain_valid
    ON domain_classifications(domain, valid_until DESC)
    WHERE valid_until > NOW();

-- Index for classification type queries
CREATE INDEX IF NOT EXISTS idx_classifications_type
    ON domain_classifications(classification_type);

-- Index for prompt lookups
CREATE INDEX IF NOT EXISTS idx_classifications_prompt
    ON domain_classifications(prompt_id);
