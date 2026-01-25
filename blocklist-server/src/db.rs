use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Database error: {0}")]
    SqlxError(#[from] sqlx::Error),
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
