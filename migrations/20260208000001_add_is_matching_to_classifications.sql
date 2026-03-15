-- Add is_matching_site column to domain_classifications table
-- This allows projections to store both positive and negative classifications
ALTER TABLE domain_classifications
ADD COLUMN is_matching_site BOOLEAN NOT NULL DEFAULT true;

-- Update existing records to have is_matching_site = true
-- (all existing records are positive matches since we only stored those)
UPDATE domain_classifications SET is_matching_site = true WHERE is_matching_site IS NULL;
