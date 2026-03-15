-- Add prompt_id foreign key to domain_classification_events table.
-- This allows events to reference prompts for full reconstructability.
-- Nullable because existing events don't have this data.
ALTER TABLE domain_classification_events
ADD COLUMN prompt_id INTEGER REFERENCES prompts(id);

-- Create index for performance when querying by prompt.
CREATE INDEX idx_domain_classification_events_prompt_id
ON domain_classification_events(prompt_id);
