-- Create domain_classification_events table - immutable event log
CREATE TYPE classification_action AS ENUM ('queued', 'classifying', 'classified', 'error');

CREATE TABLE IF NOT EXISTS domain_classification_events (
    id SERIAL PRIMARY KEY,
    domain TEXT NOT NULL,
    action classification_action NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    action_data JSONB NOT NULL DEFAULT '{}'::jsonb
);

-- Index for efficient domain lookups (DESC for latest-first queries)
CREATE INDEX IF NOT EXISTS idx_events_domain_created
    ON domain_classification_events(domain, created_at DESC);

-- Index for querying by action type
CREATE INDEX IF NOT EXISTS idx_events_action
    ON domain_classification_events(action);
