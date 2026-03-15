-- Add 'dns_nxdomain' to classification_source_type.
-- When the queue-processor resolves a domain and receives NXDOMAIN, it writes
-- a synthetic "not matching" classification with this source type rather than
-- invoking the LLM.  Each NXDOMAIN result creates its own (non-deduped) source
-- row, mirroring the 'admin' pattern, because the check is per-processing-run.
ALTER TYPE classification_source_type ADD VALUE 'dns_nxdomain';
