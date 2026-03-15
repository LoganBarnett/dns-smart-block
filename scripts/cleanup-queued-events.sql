-- One-time cleanup: tombstone all domains stuck in "queued" state.
--
-- "queued" is being dropped from the event lifecycle. Domains whose latest
-- event is "queued" have no terminal state and would be confusing to debug.
-- Writing an "error" event gives them a clean terminal state and ensures they
-- get reclassified the next time they appear in DNS traffic.
--
-- Safe to run multiple times: the WHERE clause only matches domains whose
-- current latest event is still "queued", so re-running after the first pass
-- is a no-op.
--
-- Run as: psql -U <user> <db> -f cleanup-queued-events.sql

WITH latest_events AS (
  SELECT DISTINCT ON (domain)
    domain,
    action
  FROM domain_classification_events
  ORDER BY domain, created_at DESC
)
INSERT INTO domain_classification_events (domain, action, action_data, created_at)
SELECT
  domain,
  'error'::classification_action,
  '{"reason": "abandoned in queued state; cleaned up 2026-02-19"}'::jsonb,
  NOW()
FROM latest_events
WHERE action = 'queued';
