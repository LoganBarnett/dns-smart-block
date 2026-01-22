use sqlx::{PgPool, Row};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Database error: {0}")]
    SqlxError(#[from] sqlx::Error),
}

/// Check if a domain should be queued based on latest event
/// Returns true if domain should be queued, false if it should be skipped
pub async fn should_queue_domain(
    pool: &PgPool,
    domain: &str,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        r#"
        SELECT action::text, action_data, created_at
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
        None => {
            // No events for this domain - queue it
            Ok(true)
        }
        Some(row) => {
            let action: String = row.try_get("action")?;
            match action.as_str() {
                "queued" | "classifying" => {
                    // Domain is already queued or being classified - skip
                    Ok(false)
                }
                "classified" => {
                    // Check if classification is still valid
                    let has_valid = sqlx::query(
                        r#"
                        SELECT COUNT(*) as count
                        FROM domain_classifications
                        WHERE domain = $1 AND valid_until > NOW()
                        "#,
                    )
                    .bind(domain)
                    .fetch_one(pool)
                    .await?;

                    let count: i64 = has_valid.try_get("count")?;
                    // If no valid classification, queue again
                    Ok(count == 0)
                }
                "error" => {
                    // Previous attempt failed - could queue again
                    // For now, let's not retry errors automatically
                    Ok(false)
                }
                _ => {
                    // Unknown action - queue it to be safe
                    Ok(true)
                }
            }
        }
    }
}

/// Insert a "queued" event for a domain
pub async fn insert_queued_event(
    pool: &PgPool,
    domain: &str,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO domain_classification_events (domain, action, action_data, created_at)
        VALUES ($1, 'queued'::classification_action, '{}'::jsonb, NOW())
        "#,
    )
    .bind(domain)
    .execute(pool)
    .await?;

    Ok(())
}
