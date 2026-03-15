use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sqlx::{PgPool, Postgres, Row};

use super::error::DbError;
use super::models::{
  ClassificationEventInsert, ClassificationInsert, ClassificationSource,
  DomainUpsert, PromptInsert,
};

/// Insert a domain_classification_event.  Accepts any Postgres executor so
/// callers can pass either a pool or an in-flight transaction.
pub async fn insert_event(
  executor: impl sqlx::Executor<'_, Database = Postgres>,
  domain: &str,
  action: &str,
  action_data: serde_json::Value,
  source_id: Option<i32>,
) -> Result<(), DbError> {
  ClassificationEventInsert {
    domain: domain.to_string(),
    action: action.to_string(),
    action_data,
    source_id,
  }
  .insert(executor)
  .await?;
  Ok(())
}

/// Store a classification result: ensures the prompt, ensures the
/// classification source, upserts the domain, and inserts a new projection
/// row — all within a single transaction.
///
/// Returns the source_id so callers can stamp audit events with the same
/// provenance (e.g. the classified event in the queue-processor).
///
/// This is the canonical way to persist a successful LLM classification.
pub async fn classification_store(
  pool: &PgPool,
  domain: &str,
  classification_type: &str,
  is_matching_site: bool,
  confidence: f64,
  reasoning: &str,
  model: &str,
  prompt_content: &str,
  prompt_hash: &str,
  ttl_days: i64,
) -> Result<i32, DbError> {
  let mut tx = pool.begin().await?;

  let prompt = PromptInsert {
    content: prompt_content.to_string(),
    hash: prompt_hash.to_string(),
  };
  let prompt_id = prompt.ensure(&mut tx).await?;

  let source_id =
    ClassificationSource::ensure_for_prompt(prompt_id, &mut tx).await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let valid_on = Utc::now();
  let valid_until = valid_on + Duration::days(ttl_days);

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site,
    confidence: confidence as f32,
    reasoning: Some(reasoning.to_string()),
    valid_on,
    valid_until,
    model: model.to_string(),
    source_id: Some(source_id),
  }
  .insert(&mut tx)
  .await?;

  tx.commit().await?;
  Ok(source_id)
}

/// Writes a synthetic "not matching" classification for a domain that matches
/// an exclude suffix, without invoking the LLM.  The domain upsert, source
/// ensure, projection insert, and audit event are all written atomically in a
/// single transaction so the record is always consistent.
pub async fn apply_exclude_rule(
  domain: &str,
  classification_type: &str,
  matched_suffix: &str,
  pool: &PgPool,
  ttl_days: i64,
) -> Result<(), DbError> {
  let reasoning = format!("Excluded by suffix: {}", matched_suffix);
  let now = Utc::now();

  let mut tx = pool.begin().await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let source_id = ClassificationSource::ensure_exclude_rule(
    "config_exclude_rule",
    matched_suffix,
    &mut tx,
  )
  .await?;

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site: false,
    confidence: 1.0,
    reasoning: Some(reasoning.clone()),
    valid_on: now,
    valid_until: now + Duration::days(ttl_days),
    model: "exclude-rule".to_string(),
    source_id: Some(source_id),
  }
  .insert(&mut tx)
  .await?;

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": classification_type,
      "is_matching_site": false,
      "confidence": 1.0,
      "reasoning": reasoning,
      "exclusion_suffix": matched_suffix,
      "model": "exclude-rule",
    }),
    Some(source_id),
  )
  .await?;

  tx.commit().await?;
  Ok(())
}

/// Write a manual admin classification for a domain.  Creates an `admin`
/// source row tied to the given user, inserts a projection, and appends a
/// `classified` event — all atomically in one transaction.
///
/// Returns the `source_id` of the newly created admin source.
pub async fn apply_admin_classification(
  pool: &PgPool,
  domain: &str,
  classification_type: &str,
  is_matching_site: bool,
  confidence: f64,
  reasoning: &str,
  user_id: i32,
  ttl_days: i64,
) -> Result<i32, DbError> {
  let mut tx = pool.begin().await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let source_id = ClassificationSource::insert_admin(user_id, &mut tx).await?;

  let now = Utc::now();

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site,
    confidence: confidence as f32,
    reasoning: Some(reasoning.to_string()),
    valid_on: now,
    valid_until: now + Duration::days(ttl_days),
    model: "admin".to_string(),
    source_id: Some(source_id),
  }
  .insert(&mut tx)
  .await?;

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": classification_type,
      "is_matching_site": is_matching_site,
      "confidence": confidence,
      "reasoning": reasoning,
      "model": "admin",
    }),
    Some(source_id),
  )
  .await?;

  tx.commit().await?;
  Ok(source_id)
}

/// Rebuild the `domain_classifications` projection from the event log.
/// Processes all "classified" events and recreates projection rows.
/// Returns the count of projections written.
pub async fn rebuild_projections_from_events(
  pool: &PgPool,
  ttl_days: i64,
) -> Result<i64, DbError> {
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
            source_id,
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
        source_id,
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

  sqlx::query("DELETE FROM domain_classifications")
    .execute(&mut *tx)
    .await?;

  for row in rows {
    let domain: String = row.try_get("domain")?;
    let classification_type: String = row.try_get("classification_type")?;
    let is_matching_site: bool = row.try_get("is_matching_site")?;
    let confidence: f32 = row.try_get("confidence")?;
    let reasoning: Option<String> = row.try_get("reasoning")?;
    let model: String = row.try_get("model")?;
    let source_id: Option<i32> = row.try_get("source_id")?;
    let event_created_at: DateTime<Utc> = row.try_get("created_at")?;

    let valid_on = event_created_at;
    let valid_until = valid_on + Duration::days(ttl_days);

    DomainUpsert {
      domain: domain.clone(),
    }
    .upsert(&mut tx)
    .await?;

    ClassificationInsert {
      domain,
      classification_type,
      is_matching_site,
      confidence,
      reasoning,
      valid_on,
      valid_until,
      model,
      source_id,
    }
    .insert_with_created_at(&mut tx, event_created_at)
    .await?;

    count += 1;
  }

  tx.commit().await?;
  Ok(count)
}
