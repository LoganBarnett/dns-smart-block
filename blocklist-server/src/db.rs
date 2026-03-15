use chrono::{DateTime, Utc};
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
    /// Total count of currently valid classifications (all types).
    pub current_classifications_total: i64,
    /// Total unique domains ever seen.
    pub domains_seen_total: i64,
    /// Count of classification events by action type.
    pub events_by_action: HashMap<String, i64>,
    /// Total classifications ever created per type (cumulative).
    pub classifications_created_by_type: HashMap<String, i64>,
    /// Total classifications ever created (all types).
    pub classifications_created_total: i64,
}

/// Get all blocked domains for a given classification type at a specific time
/// Returns domains where the classification is valid at the given time
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

    // Get total unique domains seen.
    let domains_seen_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
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

    Ok(MetricsStats {
        current_classifications_by_type,
        current_classifications_total: current_total,
        domains_seen_total,
        events_by_action,
        classifications_created_by_type,
        classifications_created_total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    async fn setup_test_db() -> PgPool {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/dns_smart_block_test".to_string());

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
        let gaming_domains = get_blocked_domains(&pool, "gaming", None).await.unwrap();
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
        let domains_future = get_blocked_domains(&pool, "gaming", Some(future_start + Duration::hours(1)))
            .await
            .unwrap();
        assert_eq!(domains_future.len(), 1);
        assert!(domains_future.contains(&"future.com".to_string()));
    }
}
