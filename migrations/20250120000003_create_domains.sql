-- Create domains table - projection of domains we've seen
CREATE TABLE IF NOT EXISTS domains (
    domain TEXT PRIMARY KEY,
    last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for time-based queries
CREATE INDEX IF NOT EXISTS idx_domains_last_updated
    ON domains(last_updated DESC);
