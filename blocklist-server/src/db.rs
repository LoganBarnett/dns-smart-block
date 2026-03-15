use chrono::{DateTime, Duration, Utc};
use dns_smart_block_common::db_models::{ClassificationInsert, DomainUpsert};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
  #[error("Database error: {0}")]
  SqlxError(#[from] sqlx::Error),
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

/// Get all blocked domains for a given classification type at a specific time
/// Returns domains where the classification is valid at the given time and is_matching_site = true
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

  let domains = rows
    .into_iter()
    .map(|row| -> Result<String, DbError> {
      Ok(row.try_get::<String, _>("domain")?)
    })
    .collect::<Result<Vec<_>, _>>()?;

  Ok(domains)
}

/// Get comprehensive metrics statistics from the database.
pub async fn get_metrics_stats(pool: &PgPool) -> Result<MetricsStats, DbError> {
  let now = Utc::now();

  // Get currently valid classifications count by type.
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

  // Get currently valid positive classifications count by type.
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

  // Get currently valid negative classifications count by type.
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

  // Get total currently valid classifications.
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

  // Get total currently valid positive classifications.
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

  // Get total currently valid negative classifications.
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

  // Get total unique domains seen.
  let domains_seen_total: i64 =
    sqlx::query_scalar("SELECT COUNT(*) FROM domains")
      .fetch_one(pool)
      .await?;

  // Get event counts by action type.
  let events_by_action_rows = sqlx::query(
    r#"
        SELECT action::text as action, COUNT(*) as count
        FROM domain_classification_events
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

  // Get cumulative classifications created by type.
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

  // Get total cumulative classifications created.
  let classifications_created_total: i64 =
    sqlx::query_scalar("SELECT COUNT(*) FROM domain_classifications")
      .fetch_one(pool)
      .await?;

  // Get count of "classified" events in the last 5 minutes by classification_type.
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
#[derive(Debug, Clone, serde::Serialize)]
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

  let classifications = rows
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
    .collect::<Result<Vec<_>, _>>()?;

  Ok(classifications)
}

/// Rebuild projections from classified events
/// This processes all "classified" events and recreates the domain_classifications table
pub async fn rebuild_projections_from_events(
  pool: &PgPool,
  ttl_days: i64,
) -> Result<i64, DbError> {
  // Get all most recent "classified" events per domain/classification_type
  // Extract the classification details from the event data
  let rows = sqlx::query(
        r#"
        WITH latest_classified_events AS (
            SELECT DISTINCT ON (domain, (action_data->>'classification_type'))
                domain,
                action_data->>'classification_type' as classification_type,
                (action_data->>'is_matching_site')::boolean as is_matching_site,
                (action_data->>'confidence')::real as confidence,
                action_data->>'reasoning' as reasoning,
                COALESCE(action_data->>'model', 'unknown') as model,
                prompt_id,
                created_at
            FROM domain_classification_events
            WHERE action = 'classified'
            ORDER BY domain, (action_data->>'classification_type'), created_at DESC
        )
        SELECT
            domain,
            classification_type,
            is_matching_site,
            confidence,
            reasoning,
            model,
            prompt_id,
            created_at
        FROM latest_classified_events
        WHERE classification_type IS NOT NULL
          AND is_matching_site IS NOT NULL
          AND confidence IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

  let mut tx = pool.begin().await?;
  let mut count = 0i64;

  // Delete existing projections (we're rebuilding from scratch)
  sqlx::query("DELETE FROM domain_classifications")
    .execute(&mut *tx)
    .await?;

  // For each classified event, create a new projection
  for row in rows {
    let domain: String = row.try_get("domain")?;
    let classification_type: String = row.try_get("classification_type")?;
    let is_matching_site: bool = row.try_get("is_matching_site")?;
    let confidence: f32 = row.try_get("confidence")?;
    let reasoning: Option<String> = row.try_get("reasoning")?;
    let model: String = row.try_get("model")?;
    let prompt_id: Option<i32> = row.try_get("prompt_id")?;
    let event_created_at: chrono::DateTime<chrono::Utc> =
      row.try_get("created_at")?;

    // Calculate validity window based on event time
    let valid_on = event_created_at;
    let valid_until = valid_on + Duration::days(ttl_days);

    // Upsert domain to ensure it exists (foreign key constraint)
    let domain_upsert = DomainUpsert {
      domain: domain.clone(),
    };
    domain_upsert.upsert(&mut tx).await?;

    // Insert projection with full event data using typed struct
    let classification = ClassificationInsert {
      domain,
      classification_type,
      is_matching_site,
      confidence,
      reasoning,
      valid_on,
      valid_until,
      model,
      prompt_id,
    };

    classification
      .insert_with_created_at(&mut tx, event_created_at)
      .await?;

    count += 1;
  }

  tx.commit().await?;

  Ok(count)
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::Duration;

  async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
      "postgresql://localhost/dns_smart_block_test".to_string()
    });

    let pool = PgPool::connect(&database_url)
      .await
      .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("../migrations")
      .run(&pool)
      .await
      .expect("Failed to run migrations");

    // Clean up test data
    sqlx::query("DELETE FROM domain_classifications")
      .execute(&pool)
      .await
      .expect("Failed to clean classifications");

    sqlx::query("DELETE FROM domains")
      .execute(&pool)
      .await
      .expect("Failed to clean domains");

    sqlx::query("DELETE FROM prompts")
      .execute(&pool)
      .await
      .expect("Failed to clean prompts");

    pool
  }

  #[tokio::test]
  #[ignore] // Requires DATABASE_URL
  async fn test_get_blocked_domains_at_current_time() {
    let pool = setup_test_db().await;

    // Insert test prompt
    sqlx::query(
      r#"
            INSERT INTO prompts (content, hash, created_at)
            VALUES ('test prompt', 'sha256:test', NOW())
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let prompt_id: i32 = sqlx::query_scalar("SELECT id FROM prompts LIMIT 1")
      .fetch_one(&pool)
      .await
      .unwrap();

    // Insert test domains
    let domains = vec!["gaming1.com", "gaming2.com", "news1.com"];
    for domain in &domains {
      sqlx::query(
        r#"
                INSERT INTO domains (domain, last_updated)
                VALUES ($1, NOW())
                "#,
      )
      .bind(domain)
      .execute(&pool)
      .await
      .unwrap();
    }

    let now = Utc::now();
    let valid_until = now + Duration::days(10);

    // Insert gaming classifications (valid now)
    for domain in &["gaming1.com", "gaming2.com"] {
      sqlx::query(
                r#"
                INSERT INTO domain_classifications (
                    domain, classification_type, confidence, valid_on, valid_until,
                    model, prompt_id, created_at
                )
                VALUES ($1, 'gaming', 0.9, $2, $3, 'test-model', $4, NOW())
                "#,
            )
            .bind(domain)
            .bind(now)
            .bind(valid_until)
            .bind(prompt_id)
            .execute(&pool)
            .await
            .unwrap();
    }

    // Insert news classification
    sqlx::query(
      r#"
            INSERT INTO domain_classifications (
                domain, classification_type, confidence, valid_on, valid_until,
                model, prompt_id, created_at
            )
            VALUES ($1, 'news', 0.95, $2, $3, 'test-model', $4, NOW())
            "#,
    )
    .bind("news1.com")
    .bind(now)
    .bind(valid_until)
    .bind(prompt_id)
    .execute(&pool)
    .await
    .unwrap();

    // Query gaming domains
    let gaming_domains =
      get_blocked_domains(&pool, "gaming", None).await.unwrap();
    assert_eq!(gaming_domains.len(), 2);
    assert!(gaming_domains.contains(&"gaming1.com".to_string()));
    assert!(gaming_domains.contains(&"gaming2.com".to_string()));

    // Query news domains
    let news_domains = get_blocked_domains(&pool, "news", None).await.unwrap();
    assert_eq!(news_domains.len(), 1);
    assert!(news_domains.contains(&"news1.com".to_string()));
  }

  #[tokio::test]
  #[ignore] // Requires DATABASE_URL
  async fn test_get_blocked_domains_excludes_expired() {
    let pool = setup_test_db().await;

    // Insert test prompt
    sqlx::query(
      r#"
            INSERT INTO prompts (content, hash, created_at)
            VALUES ('test prompt', 'sha256:test', NOW())
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let prompt_id: i32 = sqlx::query_scalar("SELECT id FROM prompts LIMIT 1")
      .fetch_one(&pool)
      .await
      .unwrap();

    // Insert domain
    sqlx::query(
      r#"
            INSERT INTO domains (domain, last_updated)
            VALUES ('expired.com', NOW())
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let now = Utc::now();
    let expired = now - Duration::days(1);

    // Insert expired classification
    sqlx::query(
      r#"
            INSERT INTO domain_classifications (
                domain, classification_type, confidence, valid_on, valid_until,
                model, prompt_id, created_at
            )
            VALUES ($1, 'gaming', 0.9, $2, $3, 'test-model', $4, NOW())
            "#,
    )
    .bind("expired.com")
    .bind(expired - Duration::days(10))
    .bind(expired)
    .bind(prompt_id)
    .execute(&pool)
    .await
    .unwrap();

    // Should not return expired domain
    let domains = get_blocked_domains(&pool, "gaming", None).await.unwrap();
    assert_eq!(domains.len(), 0);
  }

  #[tokio::test]
  #[ignore] // Requires DATABASE_URL
  async fn test_get_blocked_domains_at_specific_time() {
    let pool = setup_test_db().await;

    // Insert test prompt
    sqlx::query(
      r#"
            INSERT INTO prompts (content, hash, created_at)
            VALUES ('test prompt', 'sha256:test', NOW())
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let prompt_id: i32 = sqlx::query_scalar("SELECT id FROM prompts LIMIT 1")
      .fetch_one(&pool)
      .await
      .unwrap();

    // Insert domain
    sqlx::query(
      r#"
            INSERT INTO domains (domain, last_updated)
            VALUES ('future.com', NOW())
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let now = Utc::now();
    let future_start = now + Duration::days(2);
    let future_end = now + Duration::days(12);

    // Insert classification that starts in the future
    sqlx::query(
      r#"
            INSERT INTO domain_classifications (
                domain, classification_type, confidence, valid_on, valid_until,
                model, prompt_id, created_at
            )
            VALUES ($1, 'gaming', 0.9, $2, $3, 'test-model', $4, NOW())
            "#,
    )
    .bind("future.com")
    .bind(future_start)
    .bind(future_end)
    .bind(prompt_id)
    .execute(&pool)
    .await
    .unwrap();

    // Should not return at current time
    let domains_now = get_blocked_domains(&pool, "gaming", None).await.unwrap();
    assert_eq!(domains_now.len(), 0);

    // Should return at future time
    let domains_future = get_blocked_domains(
      &pool,
      "gaming",
      Some(future_start + Duration::hours(1)),
    )
    .await
    .unwrap();
    assert_eq!(domains_future.len(), 1);
    assert!(domains_future.contains(&"future.com".to_string()));
  }

  #[tokio::test]
  async fn test_rebuild_projections_from_events() {
    let test_db = dns_smart_block_common::test_db::TestDb::new()
      .expect("Failed to setup test database");
    let pool = test_db.pool().await.expect("Failed to get pool");

    // Clean up test data (in correct order for foreign key constraints)
    sqlx::query("DELETE FROM domain_classification_events")
      .execute(&pool)
      .await
      .expect("Failed to clean events");
    sqlx::query("DELETE FROM domain_classifications")
      .execute(&pool)
      .await
      .expect("Failed to clean classifications");
    sqlx::query("DELETE FROM domains")
      .execute(&pool)
      .await
      .expect("Failed to clean domains");
    sqlx::query("DELETE FROM prompts")
      .execute(&pool)
      .await
      .expect("Failed to clean prompts");

    // Insert test prompts
    sqlx::query(
      r#"
            INSERT INTO prompts (id, content, hash, created_at)
            VALUES (1, 'test prompt 1', 'sha256:test1', NOW())
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
      r#"
            INSERT INTO prompts (id, content, hash, created_at)
            VALUES (2, 'test prompt 2', 'sha256:test2', NOW())
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert older classified event WITHOUT model field (simulating legacy event)
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, prompt_id, created_at
            )
            VALUES (
                'old-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": true, "confidence": 0.9, "reasoning": "Old event without model"}',
                1,
                NOW() - INTERVAL '2 days'
            )
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert newer classified event WITH model field
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, prompt_id, created_at
            )
            VALUES (
                'new-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": true, "confidence": 0.85, "reasoning": "New event with model", "model": "test-model-v1"}',
                2,
                NOW() - INTERVAL '1 day'
            )
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert negative classification event
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, prompt_id, created_at
            )
            VALUES (
                'non-gaming.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": false, "confidence": 0.95, "reasoning": "Not a gaming site", "model": "test-model-v1"}',
                2,
                NOW() - INTERVAL '1 day'
            )
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert multiple events for same domain (should take latest)
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, prompt_id, created_at
            )
            VALUES (
                'updated-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": true, "confidence": 0.7, "reasoning": "First classification", "model": "test-model-v1"}',
                1,
                NOW() - INTERVAL '3 days'
            )
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, prompt_id, created_at
            )
            VALUES (
                'updated-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": false, "confidence": 0.92, "reasoning": "Updated classification", "model": "test-model-v2"}',
                2,
                NOW() - INTERVAL '1 day'
            )
            "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Rebuild projections
    let count = rebuild_projections_from_events(&pool, 7).await.unwrap();
    assert_eq!(count, 4); // Should create 4 projections

    // Verify old event without model got 'unknown' as model
    let old_classification: (String, bool, f32, String) = sqlx::query_as(
      r#"
            SELECT model, is_matching_site, confidence, reasoning
            FROM domain_classifications
            WHERE domain = 'old-domain.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(old_classification.0, "unknown");
    assert_eq!(old_classification.1, true);
    assert_eq!(old_classification.2, 0.9);
    assert_eq!(old_classification.3, "Old event without model");

    // Verify new event with model preserved the model value
    let new_classification: (String, bool, f32, String) = sqlx::query_as(
      r#"
            SELECT model, is_matching_site, confidence, reasoning
            FROM domain_classifications
            WHERE domain = 'new-domain.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(new_classification.0, "test-model-v1");
    assert_eq!(new_classification.1, true);
    assert_eq!(new_classification.2, 0.85);
    assert_eq!(new_classification.3, "New event with model");

    // Verify negative classification was created
    let negative_classification: (bool, f32) = sqlx::query_as(
      r#"
            SELECT is_matching_site, confidence
            FROM domain_classifications
            WHERE domain = 'non-gaming.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(negative_classification.0, false);
    assert_eq!(negative_classification.1, 0.95);

    // Verify updated domain used latest event
    let updated_classification: (bool, f32, String, String) = sqlx::query_as(
      r#"
            SELECT is_matching_site, confidence, reasoning, model
            FROM domain_classifications
            WHERE domain = 'updated-domain.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(updated_classification.0, false); // Latest is negative
    assert_eq!(updated_classification.1, 0.92);
    assert_eq!(updated_classification.2, "Updated classification");
    assert_eq!(updated_classification.3, "test-model-v2");

    // Verify domains table was populated
    let domain_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
      .fetch_one(&pool)
      .await
      .unwrap();
    assert_eq!(domain_count, 4);
  }
}
