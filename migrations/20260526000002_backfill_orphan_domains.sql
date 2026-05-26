-- Backfill `domains` rows for event-domains that never received a row.
--
-- Before the matching code change (`ClassificationEventInsert.insert`
-- upserting the `domains` row via CTE), several paths wrote to
-- `domain_classification_events` without ever upserting the `domains`
-- row: the queue-processor `classifying` and `error` event inserts, and
-- `DomainRequeue.requeue`.  Domains that errored every classification
-- attempt — or were requeued by an admin without an earlier successful
-- classification — therefore stayed invisible to the `domains` table.
--
-- On the production silicon instance this gap was ~2129 domains (~4% of
-- the unique event-domain set), broken down as:
--   error       : 1112 (queue-processor error path)
--   queued      :  851 (DomainRequeue.requeue)
--   classifying :   96 (queue-processor classifying event)
--   classified  :   68 (pre-DomainUpsert historical)
--
-- This one-shot backfill closes the existing gap.  Going forward the
-- invariant is enforced at the event-insert layer; no further drift is
-- possible from the application side.
--
-- `last_updated` is set to the most recent event timestamp per domain so
-- the backfilled row reflects when we actually last saw the domain
-- rather than the migration runtime.
INSERT INTO domains (domain, last_updated)
SELECT
  e.domain,
  MAX(e.created_at) AS last_updated
FROM domain_classification_events e
LEFT JOIN domains d ON d.domain = e.domain
WHERE d.domain IS NULL
GROUP BY e.domain
ON CONFLICT (domain) DO NOTHING;
