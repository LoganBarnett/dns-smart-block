-- Make prompt_id nullable in domain_classifications
-- The log processor doesn't have access to prompts, only the queue processor
-- does after running the classifier. Historical projections rebuilt from old
-- events may also not have prompt_id.
ALTER TABLE domain_classifications
ALTER COLUMN prompt_id DROP NOT NULL;
