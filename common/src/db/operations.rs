use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Postgres, Row};
use std::collections::{HashMap, HashSet};

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
/// Sentinel `valid_until` used when a classification should never expire.
/// Approximately year 3026 — far enough to be "forever" without needing a
/// nullable column or schema change.
fn never_expires_at(now: DateTime<Utc>) -> DateTime<Utc> {
  now + Duration::days(365_000)
}

pub async fn apply_admin_classification(
  pool: &PgPool,
  domain: &str,
  classification_type: &str,
  is_matching_site: bool,
  confidence: f64,
  reasoning: &str,
  user_id: i32,
  ttl_days: Option<i64>,
) -> Result<i32, DbError> {
  let mut tx = pool.begin().await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let source_id = ClassificationSource::insert_admin(user_id, &mut tx).await?;

  let now = Utc::now();
  let valid_until = match ttl_days {
    Some(d) => now + Duration::days(d),
    None => never_expires_at(now),
  };

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site,
    confidence: confidence as f32,
    reasoning: Some(reasoning.to_string()),
    valid_on: now,
    valid_until,
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

/// A single desired provisioned classification entry, as declared in NixOS
/// configuration and submitted via `dns-smart-block-cli domain reconcile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionedEntry {
  pub domain: String,
  pub classification_type: String,
  pub is_matching_site: bool,
  pub confidence: f64,
  pub reasoning: Option<String>,
}

/// Summary of what a reconcile run changed.
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileResult {
  /// Entries that were inserted or updated.
  pub upserted: usize,
  /// Entries whose active row already matched — no write needed.
  pub skipped: usize,
  /// Previously-provisioned classifications removed from the declared set.
  pub expired: usize,
}

/// Reconcile the `provisioned` classification source against a desired set.
///
/// - Entries in `desired` are upserted (insert new or expire-and-reinsert if
///   values changed).  Unchanged active rows are skipped.
/// - Active `provisioned` rows whose `(domain, classification_type)` is not
///   in `desired` are expired and an `expired` event is written.
/// - `admin`-sourced rows are never touched.
pub async fn reconcile_provisioned_classifications(
  pool: &PgPool,
  desired: &[ProvisionedEntry],
) -> Result<ReconcileResult, DbError> {
  let mut tx = pool.begin().await?;
  let now = Utc::now();

  // Ensure the singleton provisioned source row exists.
  let source_id = ClassificationSource::ensure_provisioned(&mut tx).await?;

  // Collect all currently-active provisioned rows.
  let active_rows = sqlx::query(
    r#"
    SELECT domain, classification_type, is_matching_site, confidence, reasoning
    FROM domain_classifications
    WHERE source_id = $1 AND valid_until > $2
    "#,
  )
  .bind(source_id)
  .bind(now)
  .fetch_all(&mut *tx)
  .await?;

  // Build a lookup: (domain, classification_type) → (is_matching_site, confidence, reasoning)
  let mut active_map: HashMap<(String, String), (bool, f32, Option<String>)> =
    HashMap::new();
  for row in &active_rows {
    let domain: String = row.try_get("domain")?;
    let ct: String = row.try_get("classification_type")?;
    let is_matching: bool = row.try_get("is_matching_site")?;
    let confidence: f32 = row.try_get("confidence")?;
    let reasoning: Option<String> = row.try_get("reasoning")?;
    active_map.insert((domain, ct), (is_matching, confidence, reasoning));
  }

  let desired_keys: HashSet<(String, String)> = desired
    .iter()
    .map(|e| (e.domain.clone(), e.classification_type.clone()))
    .collect();

  let mut upserted = 0usize;
  let mut skipped = 0usize;

  for entry in desired {
    let key = (entry.domain.clone(), entry.classification_type.clone());

    // Skip if the active row already matches desired values.
    if let Some(&(existing_matching, existing_conf, ref existing_reasoning)) =
      active_map.get(&key)
    {
      let same_values = existing_matching == entry.is_matching_site
        && (existing_conf - entry.confidence as f32).abs() < 0.0001
        && existing_reasoning.as_deref().unwrap_or("")
          == entry.reasoning.as_deref().unwrap_or("");

      if same_values {
        skipped += 1;
        continue;
      }

      // Values changed — expire the old row before inserting a fresh one.
      sqlx::query(
        r#"
        UPDATE domain_classifications SET valid_until = $1
        WHERE domain = $2 AND classification_type = $3
          AND source_id = $4 AND valid_until > $1
        "#,
      )
      .bind(now)
      .bind(&entry.domain)
      .bind(&entry.classification_type)
      .bind(source_id)
      .execute(&mut *tx)
      .await?;
    }

    DomainUpsert {
      domain: entry.domain.clone(),
    }
    .upsert(&mut tx)
    .await?;

    ClassificationInsert {
      domain: entry.domain.clone(),
      classification_type: entry.classification_type.clone(),
      is_matching_site: entry.is_matching_site,
      confidence: entry.confidence as f32,
      reasoning: entry.reasoning.clone(),
      valid_on: now,
      valid_until: never_expires_at(now),
      model: "provisioned".to_string(),
      source_id: Some(source_id),
    }
    .insert(&mut tx)
    .await?;

    insert_event(
      &mut *tx,
      &entry.domain,
      "classified",
      json!({
        "classification_type": entry.classification_type,
        "is_matching_site": entry.is_matching_site,
        "confidence": entry.confidence,
        "reasoning": entry.reasoning.as_deref().unwrap_or(""),
        "model": "provisioned",
      }),
      Some(source_id),
    )
    .await?;

    upserted += 1;
  }

  // Expire any active provisioned rows that are no longer in the desired set.
  let mut expired = 0usize;
  for (domain, ct) in active_map.keys() {
    if !desired_keys.contains(&(domain.clone(), ct.clone())) {
      sqlx::query(
        r#"
        UPDATE domain_classifications SET valid_until = $1
        WHERE domain = $2 AND classification_type = $3
          AND source_id = $4 AND valid_until > $1
        "#,
      )
      .bind(now)
      .bind(domain)
      .bind(ct)
      .bind(source_id)
      .execute(&mut *tx)
      .await?;

      insert_event(
        &mut *tx,
        domain,
        "expired",
        json!({
          "classification_type": ct,
          "reason": "removed from provisioned configuration",
        }),
        Some(source_id),
      )
      .await?;

      expired += 1;
    }
  }

  tx.commit().await?;
  Ok(ReconcileResult {
    upserted,
    skipped,
    expired,
  })
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
