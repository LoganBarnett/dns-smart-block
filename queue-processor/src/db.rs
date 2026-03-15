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
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO domain_classification_events (domain, action, action_data, created_at)
        VALUES ($1, $2::classification_action, $3, NOW())
        "#,
    )
    .bind(domain)
    .bind(action)
    .bind(action_data)
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
            SELECT action::text
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
            AND re2.rowid <= recent_events.rowid
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
    confidence: f32,
    model: &str,
    prompt_id: i32,
    ttl_days: i64,
) -> Result<(), DbError> {
    let valid_on = Utc::now();
    let valid_until = valid_on + Duration::days(ttl_days);

    sqlx::query(
        r#"
        INSERT INTO domain_classifications (
            domain, classification_type, confidence, valid_on, valid_until, model, prompt_id, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        "#,
    )
    .bind(domain)
    .bind(classification_type)
    .bind(confidence)
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
    confidence: f64,
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
        confidence as f32,
        model,
        prompt_id,
        ttl_days,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}
