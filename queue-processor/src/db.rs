use chrono::{Duration, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Database error: {0}")]
    SqlxError(#[from] sqlx::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Insert a domain_classification_event
pub async fn insert_event(
    pool: &PgPool,
    domain: &str,
    action: &str,
    action_data: serde_json::Value,
    prompt_id: Option<i32>,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO domain_classification_events (domain, action, action_data, prompt_id, created_at)
        VALUES ($1, $2::classification_action, $3, $4, NOW())
        "#,
    )
    .bind(domain)
    .bind(action)
    .bind(action_data)
    .bind(prompt_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get the latest event for a domain
pub async fn get_latest_event(
    pool: &PgPool,
    domain: &str,
) -> Result<Option<(String, serde_json::Value)>, DbError> {
    let result = sqlx::query(
        r#"
        SELECT action::text, action_data
        FROM domain_classification_events
        WHERE domain = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(domain)
    .fetch_optional(pool)
    .await?;

    match result {
        Some(row) => {
            let action: String = row.try_get("action")?;
            let action_data: serde_json::Value = row.try_get("action_data")?;
            Ok(Some((action, action_data)))
        }
        None => Ok(None),
    }
}

/// Count consecutive "error" events for a domain.
/// Returns the number of consecutive errors, starting from the most recent
/// event.
pub async fn count_consecutive_errors(
    pool: &PgPool,
    domain: &str,
    limit: i64,
) -> Result<i64, DbError> {
    let result = sqlx::query(
        r#"
        WITH recent_events AS (
            SELECT
                action::text,
                ROW_NUMBER() OVER (ORDER BY created_at DESC) as rn
            FROM domain_classification_events
            WHERE domain = $1
            ORDER BY created_at DESC
            LIMIT $2
        )
        SELECT COUNT(*) as error_count
        FROM recent_events
        WHERE action = 'error'
        AND NOT EXISTS (
            SELECT 1
            FROM recent_events re2
            WHERE re2.action != 'error'
            AND re2.rn <= recent_events.rn
        )
        "#,
    )
    .bind(domain)
    .bind(limit)
    .fetch_one(pool)
    .await?;

    let count: i64 = result.try_get("error_count")?;
    Ok(count)
}

/// Ensure a prompt exists and return its ID
pub async fn ensure_prompt(
    tx: &mut Transaction<'_, Postgres>,
    content: &str,
    hash: &str,
) -> Result<i32, DbError> {
    // Try to insert, ignore conflicts
    sqlx::query(
        r#"
        INSERT INTO prompts (content, hash, created_at)
        VALUES ($1, $2, NOW())
        ON CONFLICT (hash) DO NOTHING
        "#,
    )
    .bind(content)
    .bind(hash)
    .execute(&mut **tx)
    .await?;

    // Get the ID
    let result = sqlx::query(
        r#"
        SELECT id FROM prompts WHERE hash = $1
        "#,
    )
    .bind(hash)
    .fetch_one(&mut **tx)
    .await?;

    let id: i32 = result.try_get("id")?;
    Ok(id)
}

/// Upsert a domain in the domains table
pub async fn upsert_domain(
    tx: &mut Transaction<'_, Postgres>,
    domain: &str,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO domains (domain, last_updated)
        VALUES ($1, NOW())
        ON CONFLICT (domain) DO UPDATE SET last_updated = NOW()
        "#,
    )
    .bind(domain)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Insert a domain classification
pub async fn insert_classification(
    tx: &mut Transaction<'_, Postgres>,
    domain: &str,
    classification_type: &str,
    is_matching_site: bool,
    confidence: f32,
    reasoning: &str,
    model: &str,
    prompt_id: i32,
    ttl_days: i64,
) -> Result<(), DbError> {
    let valid_on = Utc::now();
    let valid_until = valid_on + Duration::days(ttl_days);

    sqlx::query(
        r#"
        INSERT INTO domain_classifications (
            domain, classification_type, is_matching_site, confidence, reasoning, valid_on, valid_until, model, prompt_id, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
        "#,
    )
    .bind(domain)
    .bind(classification_type)
    .bind(is_matching_site)
    .bind(confidence)
    .bind(reasoning)
    .bind(valid_on)
    .bind(valid_until)
    .bind(model)
    .bind(prompt_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Update projections after a successful classification
pub async fn update_projections(
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
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    // Ensure prompt exists
    let prompt_id = ensure_prompt(&mut tx, prompt_content, prompt_hash).await?;

    // Upsert domain
    upsert_domain(&mut tx, domain).await?;

    // Insert classification
    insert_classification(
        &mut tx,
        domain,
        classification_type,
        is_matching_site,
        confidence as f32,
        reasoning,
        model,
        prompt_id,
        ttl_days,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}

/// Classification state for a specific classifier.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassifierState {
    /// Classification is current (within TTL, not in error state)
    Current,
    /// Classification exists but is expired (past valid_until)
    Expired,
    /// Most recent event is an error
    Error,
    /// No classification exists for this type
    Missing,
}

/// Get the state of all classifiers for a domain in a single query.
///
/// This is optimized to fetch all classifier states at once rather than
/// querying each one individually.
pub async fn get_classifier_states(
    pool: &PgPool,
    domain: &str,
    classification_types: &[String],
) -> Result<Vec<(String, ClassifierState)>, DbError> {
    // Build a SQL query that checks both domain_classifications and
    // domain_classification_events for each classification type.
    //
    // For each type, we check:
    // 1. Is there a valid classification (valid_until > NOW)?
    // 2. Is the most recent event an error?
    // 3. Otherwise, it's either expired or missing.

    let result = sqlx::query(
        r#"
        WITH classification_types AS (
            SELECT unnest($2::text[]) AS classification_type
        ),
        latest_classifications AS (
            SELECT DISTINCT ON (dc.classification_type)
                dc.classification_type,
                dc.valid_until,
                dc.valid_until > NOW() AS is_valid
            FROM domain_classifications dc
            WHERE dc.domain = $1
            ORDER BY dc.classification_type, dc.created_at DESC
        ),
        latest_events AS (
            SELECT DISTINCT ON (action_data->>'classification_type')
                action_data->>'classification_type' AS classification_type,
                action::text AS action
            FROM domain_classification_events
            WHERE domain = $1
              AND action_data->>'classification_type' IS NOT NULL
            ORDER BY action_data->>'classification_type', created_at DESC
        )
        SELECT
            ct.classification_type,
            lc.is_valid,
            le.action
        FROM classification_types ct
        LEFT JOIN latest_classifications lc
            ON ct.classification_type = lc.classification_type
        LEFT JOIN latest_events le
            ON ct.classification_type = le.classification_type
        "#,
    )
    .bind(domain)
    .bind(classification_types)
    .fetch_all(pool)
    .await?;

    let mut states = Vec::new();

    for row in result {
        let classification_type: String = row.try_get("classification_type")?;
        let is_valid: Option<bool> = row.try_get("is_valid").ok();
        let latest_action: Option<String> = row.try_get("action").ok();

        let state = match (is_valid, latest_action.as_deref()) {
            // Has a valid classification.
            (Some(true), _) => ClassifierState::Current,

            // Has a classification but it's expired.
            (Some(false), _) => ClassifierState::Expired,

            // No classification, but latest event is an error.
            (None, Some("error")) => ClassifierState::Error,

            // No classification and no error = missing.
            (None, _) => ClassifierState::Missing,
        };

        states.push((classification_type, state));
    }

    Ok(states)
}
