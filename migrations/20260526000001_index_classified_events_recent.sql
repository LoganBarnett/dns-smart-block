-- Partial index for `recent_classified_by_type` in `get_metrics_stats`.
--
-- The Prometheus `/metrics` endpoint runs this filter every scrape:
--   WHERE action = 'classified' AND created_at >= NOW() - INTERVAL '5 minutes'
-- Without this index, the only usable predicate index was `idx_events_action`
-- on `action` alone, which forced a parallel sequential scan over every row
-- with `action = 'classified'` to find the handful inside the 5-minute
-- window.  On a production database of ~485k events (~221k classified), this
-- consistently measured ~53ms per scrape — and with a 10s scrape interval,
-- that is ~5s/min of wasted CPU on a query that returns 0–10 rows.
--
-- A partial index keyed on `created_at` scoped to `action = 'classified'`
-- is small (~5MB at the above row counts) and reduces the query to a
-- single index range scan.  Measured payoff: 53ms → 0.1ms (~500x).
--
-- DESC ordering matches the query's "recent events" access pattern and
-- mirrors the convention used in `idx_events_domain_created`.
CREATE INDEX IF NOT EXISTS idx_events_classified_created_at
    ON domain_classification_events (created_at DESC)
    WHERE action = 'classified';
