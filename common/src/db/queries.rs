use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

use super::error::DbError;

/// Get all blocked domains for a given classification type at a specific time.
/// Returns domains where the classification is valid at the given time and
/// `is_matching_site = true`.
pub async fn get_blocked_domains(
  pool: &PgPool,
  classification_type: &str,
  at_time: Option<DateTime<Utc>>,
) -> Result<Vec<String>, DbError> {
  let check_time = at_time.unwrap_or_else(Utc::now);

  let rows = sqlx::query(
    r#"
    SELECT DISTINCT d.domain
    FROM domains d
    INNER JOIN domain_classifications dc ON d.domain = dc.domain
    WHERE dc.classification_type = $1
      AND dc.is_matching_site = true
      AND dc.valid_on <= $2
      AND dc.valid_until > $2
    ORDER BY d.domain ASC
    "#,
  )
  .bind(classification_type)
  .bind(check_time)
  .fetch_all(pool)
  .await?;

  rows
    .into_iter()
    .map(|row| Ok(row.try_get::<String, _>("domain")?))
    .collect()
}

/// Statistics about classifications in the database.
#[derive(Debug, Clone)]
pub struct MetricsStats {
  /// Count of currently valid classifications per type.
  pub current_classifications_by_type: HashMap<String, i64>,
  /// Count of currently valid positive classifications per type.
  pub current_positive_by_type: HashMap<String, i64>,
  /// Count of currently valid negative classifications per type.
  pub current_negative_by_type: HashMap<String, i64>,
  /// Total count of currently valid classifications (all types).
  pub current_classifications_total: i64,
  /// Total count of currently valid positive classifications.
  pub current_positive_total: i64,
  /// Total count of currently valid negative classifications.
  pub current_negative_total: i64,
  /// Total unique domains ever seen.
  pub domains_seen_total: i64,
  /// Count of classification events by action type.
  pub events_by_action: HashMap<String, i64>,
  /// Total classifications ever created per type (cumulative).
  pub classifications_created_by_type: HashMap<String, i64>,
  /// Total classifications ever created (all types).
  pub classifications_created_total: i64,
  /// Count of "classified" events in the last 5 minutes per classification_type.
  pub recent_classified_by_type: HashMap<String, i64>,
}

/// Get comprehensive metrics statistics from the database.
pub async fn get_metrics_stats(pool: &PgPool) -> Result<MetricsStats, DbError> {
  let now = Utc::now();

  let current_by_type_rows = sqlx::query(
    r#"
    SELECT classification_type, COUNT(DISTINCT domain) as count
    FROM domain_classifications
    WHERE valid_on <= $1 AND valid_until > $1
    GROUP BY classification_type
    "#,
  )
  .bind(now)
  .fetch_all(pool)
  .await?;

  let mut current_classifications_by_type = HashMap::new();
  for row in current_by_type_rows {
    let classification_type: String = row.try_get("classification_type")?;
    let count: i64 = row.try_get("count")?;
    current_classifications_by_type.insert(classification_type, count);
  }

  let current_positive_rows = sqlx::query(
    r#"
    SELECT classification_type, COUNT(DISTINCT domain) as count
    FROM domain_classifications
    WHERE valid_on <= $1 AND valid_until > $1 AND is_matching_site = true
    GROUP BY classification_type
    "#,
  )
  .bind(now)
  .fetch_all(pool)
  .await?;

  let mut current_positive_by_type = HashMap::new();
  for row in current_positive_rows {
    let classification_type: String = row.try_get("classification_type")?;
    let count: i64 = row.try_get("count")?;
    current_positive_by_type.insert(classification_type, count);
  }

  let current_negative_rows = sqlx::query(
    r#"
    SELECT classification_type, COUNT(DISTINCT domain) as count
    FROM domain_classifications
    WHERE valid_on <= $1 AND valid_until > $1 AND is_matching_site = false
    GROUP BY classification_type
    "#,
  )
  .bind(now)
  .fetch_all(pool)
  .await?;

  let mut current_negative_by_type = HashMap::new();
  for row in current_negative_rows {
    let classification_type: String = row.try_get("classification_type")?;
    let count: i64 = row.try_get("count")?;
    current_negative_by_type.insert(classification_type, count);
  }

  let current_total: i64 = sqlx::query_scalar(
    r#"
        SELECT COUNT(DISTINCT domain)
        FROM domain_classifications
        WHERE valid_on <= $1 AND valid_until > $1
        "#,
  )
  .bind(now)
  .fetch_one(pool)
  .await?;

  let current_positive_total: i64 = sqlx::query_scalar(
    r#"
        SELECT COUNT(DISTINCT domain)
        FROM domain_classifications
        WHERE valid_on <= $1 AND valid_until > $1 AND is_matching_site = true
        "#,
  )
  .bind(now)
  .fetch_one(pool)
  .await?;

  let current_negative_total: i64 = sqlx::query_scalar(
    r#"
        SELECT COUNT(DISTINCT domain)
        FROM domain_classifications
        WHERE valid_on <= $1 AND valid_until > $1 AND is_matching_site = false
        "#,
  )
  .bind(now)
  .fetch_one(pool)
  .await?;

  let domains_seen_total: i64 =
    sqlx::query_scalar("SELECT COUNT(*) FROM domains")
      .fetch_one(pool)
      .await?;

  // For each domain, only the latest event counts.
  let events_by_action_rows = sqlx::query(
    r#"
        SELECT action::text as action, COUNT(*) as count
        FROM (
          SELECT DISTINCT ON (domain) action
          FROM domain_classification_events
          ORDER BY domain, created_at DESC
        ) latest
        GROUP BY action
        "#,
  )
  .fetch_all(pool)
  .await?;

  let mut events_by_action = HashMap::new();
  for row in events_by_action_rows {
    let action: String = row.try_get("action")?;
    let count: i64 = row.try_get("count")?;
    events_by_action.insert(action, count);
  }

  let created_by_type_rows = sqlx::query(
    r#"
        SELECT classification_type, COUNT(*) as count
        FROM domain_classifications
        GROUP BY classification_type
        "#,
  )
  .fetch_all(pool)
  .await?;

  let mut classifications_created_by_type = HashMap::new();
  for row in created_by_type_rows {
    let classification_type: String = row.try_get("classification_type")?;
    let count: i64 = row.try_get("count")?;
    classifications_created_by_type.insert(classification_type, count);
  }

  let classifications_created_total: i64 =
    sqlx::query_scalar("SELECT COUNT(*) FROM domain_classifications")
      .fetch_one(pool)
      .await?;

  let five_minutes_ago = Utc::now() - Duration::minutes(5);
  let recent_classified_rows = sqlx::query(
    r#"
        SELECT action_data->>'classification_type' as classification_type, COUNT(*) as count
        FROM domain_classification_events
        WHERE action = 'classified'::classification_action
          AND created_at >= $1
          AND action_data->>'classification_type' IS NOT NULL
        GROUP BY action_data->>'classification_type'
        "#,
  )
  .bind(five_minutes_ago)
  .fetch_all(pool)
  .await?;

  let mut recent_classified_by_type = HashMap::new();
  for row in recent_classified_rows {
    let classification_type: String = row.try_get("classification_type")?;
    let count: i64 = row.try_get("count")?;
    recent_classified_by_type.insert(classification_type, count);
  }

  Ok(MetricsStats {
    current_classifications_by_type,
    current_positive_by_type,
    current_negative_by_type,
    current_classifications_total: current_total,
    current_positive_total,
    current_negative_total,
    domains_seen_total,
    events_by_action,
    classifications_created_by_type,
    classifications_created_total,
    recent_classified_by_type,
  })
}

/// Classification details for diagnostics endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationDetail {
  pub domain: String,
  pub classification_type: String,
  pub is_matching_site: bool,
  pub confidence: f32,
  pub reasoning: Option<String>,
  pub model: String,
  pub valid_on: DateTime<Utc>,
  pub valid_until: DateTime<Utc>,
  pub created_at: DateTime<Utc>,
}

/// Get all current valid classifications with details (for diagnostics).
/// Can optionally filter by classification_type.
pub async fn get_classifications(
  pool: &PgPool,
  classification_type: Option<&str>,
) -> Result<Vec<ClassificationDetail>, DbError> {
  let now = Utc::now();

  let rows = if let Some(ct) = classification_type {
    sqlx::query(
      r#"
            SELECT
                dc.domain,
                dc.classification_type,
                dc.is_matching_site,
                dc.confidence,
                dc.reasoning,
                dc.model,
                dc.valid_on,
                dc.valid_until,
                dc.created_at
            FROM domain_classifications dc
            WHERE dc.classification_type = $1
              AND dc.valid_on <= $2
              AND dc.valid_until > $2
            ORDER BY dc.created_at DESC
            "#,
    )
    .bind(ct)
    .bind(now)
    .fetch_all(pool)
    .await?
  } else {
    sqlx::query(
      r#"
            SELECT
                dc.domain,
                dc.classification_type,
                dc.is_matching_site,
                dc.confidence,
                dc.reasoning,
                dc.model,
                dc.valid_on,
                dc.valid_until,
                dc.created_at
            FROM domain_classifications dc
            WHERE dc.valid_on <= $1
              AND dc.valid_until > $1
            ORDER BY dc.created_at DESC
            "#,
    )
    .bind(now)
    .fetch_all(pool)
    .await?
  };

  rows
    .into_iter()
    .map(|row| -> Result<ClassificationDetail, DbError> {
      Ok(ClassificationDetail {
        domain: row.try_get("domain")?,
        classification_type: row.try_get("classification_type")?,
        is_matching_site: row.try_get("is_matching_site")?,
        confidence: row.try_get("confidence")?,
        reasoning: row.try_get("reasoning")?,
        model: row.try_get("model")?,
        valid_on: row.try_get("valid_on")?,
        valid_until: row.try_get("valid_until")?,
        created_at: row.try_get("created_at")?,
      })
    })
    .collect()
}

/// Classification row with joined provenance from `classification_sources`.
#[derive(Debug, Clone, Serialize)]
pub struct ClassificationWithSource {
  pub classification_type: String,
  pub is_matching_site: bool,
  pub confidence: f32,
  pub reasoning: Option<String>,
  pub model: String,
  pub valid_on: DateTime<Utc>,
  pub valid_until: DateTime<Utc>,
  pub is_current: bool,
  pub created_at: DateTime<Utc>,
  pub source_type: Option<String>,
  pub source_label: Option<String>,
  pub source_user_id: Option<i32>,
}

/// A single event from the classification log, with optional source provenance.
#[derive(Debug, Clone, Serialize)]
pub struct EventSummary {
  pub action: String,
  pub action_data: serde_json::Value,
  pub source_type: Option<String>,
  pub created_at: DateTime<Utc>,
}

/// Full status of a domain: whether it exists, all its classifications with
/// source provenance, and its recent event history.
#[derive(Debug, Clone, Serialize)]
pub struct DomainStatus {
  pub domain: String,
  pub exists: bool,
  pub classifications: Vec<ClassificationWithSource>,
  pub recent_events: Vec<EventSummary>,
}

/// Return full status for a domain: current and historical classifications
/// (with source provenance) plus the 20 most recent events.
pub async fn get_domain_status(
  pool: &PgPool,
  domain: &str,
) -> Result<DomainStatus, DbError> {
  let exists: bool = sqlx::query_scalar(
    "SELECT EXISTS(SELECT 1 FROM domains WHERE domain = $1)",
  )
  .bind(domain)
  .fetch_one(pool)
  .await?;

  let classification_rows = sqlx::query(
    r#"
    SELECT
      dc.classification_type,
      dc.is_matching_site,
      dc.confidence,
      dc.reasoning,
      dc.model,
      dc.valid_on,
      dc.valid_until,
      dc.valid_until > NOW() AS is_current,
      dc.created_at,
      cs.source_type::text AS source_type,
      cs.label AS source_label,
      cs.user_id AS source_user_id
    FROM domain_classifications dc
    LEFT JOIN classification_sources cs ON dc.source_id = cs.id
    WHERE dc.domain = $1
    ORDER BY dc.created_at DESC
    "#,
  )
  .bind(domain)
  .fetch_all(pool)
  .await?;

  let classifications = classification_rows
    .into_iter()
    .map(|row| -> Result<ClassificationWithSource, DbError> {
      Ok(ClassificationWithSource {
        classification_type: row.try_get("classification_type")?,
        is_matching_site: row.try_get("is_matching_site")?,
        confidence: row.try_get("confidence")?,
        reasoning: row.try_get("reasoning")?,
        model: row.try_get("model")?,
        valid_on: row.try_get("valid_on")?,
        valid_until: row.try_get("valid_until")?,
        is_current: row.try_get("is_current")?,
        created_at: row.try_get("created_at")?,
        source_type: row.try_get("source_type")?,
        source_label: row.try_get("source_label")?,
        source_user_id: row.try_get("source_user_id")?,
      })
    })
    .collect::<Result<Vec<_>, _>>()?;

  let event_rows = sqlx::query(
    r#"
    SELECT
      dce.action::text AS action,
      dce.action_data,
      cs.source_type::text AS source_type,
      dce.created_at
    FROM domain_classification_events dce
    LEFT JOIN classification_sources cs ON dce.source_id = cs.id
    WHERE dce.domain = $1
    ORDER BY dce.created_at DESC
    LIMIT 20
    "#,
  )
  .bind(domain)
  .fetch_all(pool)
  .await?;

  let recent_events = event_rows
    .into_iter()
    .map(|row| -> Result<EventSummary, DbError> {
      Ok(EventSummary {
        action: row.try_get("action")?,
        action_data: row.try_get("action_data")?,
        source_type: row.try_get("source_type")?,
        created_at: row.try_get("created_at")?,
      })
    })
    .collect::<Result<Vec<_>, _>>()?;

  Ok(DomainStatus {
    domain: domain.to_string(),
    exists,
    classifications,
    recent_events,
  })
}
