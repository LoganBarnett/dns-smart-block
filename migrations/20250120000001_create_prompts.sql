-- Create prompts table for storing LLM prompts with deduplication via hash
CREATE TABLE IF NOT EXISTS prompts (
    id SERIAL PRIMARY KEY,
    content TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for efficient hash lookups
CREATE INDEX IF NOT EXISTS idx_prompts_hash ON prompts(hash);
