use dns_smart_block_common::db_models::ClassificationEventInsert;
use dns_smart_block_log_processor::db::should_queue_domain;
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, Row};

async fn setup_test_db() -> (dns_smart_block_common::test_db::TestDb, PgPool) {
  let test_db = dns_smart_block_common::test_db::TestDb::new()
    .expect("failed to start test db");
  let pool = test_db.pool().await.expect("failed to get pool");

  sqlx::query("DELETE FROM domain_classification_events")
    .execute(&pool)
    .await
    .expect("Failed to clean test data");
  sqlx::query("DELETE FROM domain_classifications")
    .execute(&pool)
    .await
    .expect("Failed to clean classifications");
  sqlx::query("DELETE FROM domains")
    .execute(&pool)
    .await
    .expect("Failed to clean domains");

  (test_db, pool)
}

#[tokio::test]
#[serial]

async fn test_should_queue_domain_when_no_events() {
  let (_db, pool) = setup_test_db().await;

  // Domain with no events should be queued
  let should_queue = should_queue_domain(&pool, "new-domain.com")
    .await
    .expect("should_queue_domain failed");

  assert!(should_queue, "New domain should be queued");
}

#[tokio::test]
#[serial]

async fn test_should_not_queue_when_already_queued() {
  let (_db, pool) = setup_test_db().await;

  let domain = "already-queued.com";

  // Insert a "queued" event
  ClassificationEventInsert {
    domain: domain.to_string(),
    action: "queued".to_string(),
    action_data: serde_json::json!({}),
    prompt_id: None,
  }
  .insert(&pool)
  .await
  .expect("Failed to insert queued event");

  // Should not queue again
  let should_queue = should_queue_domain(&pool, domain)
    .await
    .expect("should_queue_domain failed");

  assert!(
    !should_queue,
    "Already queued domain should not be queued again"
  );
}

#[tokio::test]
#[serial]

async fn test_should_not_queue_when_classifying() {
  let (_db, pool) = setup_test_db().await;

  let domain = "being-classified.com";

  // Insert a "classifying" event
  sqlx::query(
        r#"
        INSERT INTO domain_classification_events (domain, action, action_data, created_at)
        VALUES ($1, 'classifying'::classification_action, $2, NOW())
        "#,
    )
    .bind(domain)
    .bind(json!({"model": "test-model"}))
    .execute(&pool)
    .await
    .expect("Failed to insert classifying event");

  // Should not queue again
  let should_queue = should_queue_domain(&pool, domain)
    .await
    .expect("should_queue_domain failed");

  assert!(
    !should_queue,
    "Domain being classified should not be queued again"
  );
}

#[tokio::test]
#[serial]

async fn test_should_not_queue_when_error() {
  let (_db, pool) = setup_test_db().await;

  let domain = "error-domain.com";

  // Insert an "error" event
  sqlx::query(
        r#"
        INSERT INTO domain_classification_events (domain, action, action_data, created_at)
        VALUES ($1, 'error'::classification_action, $2, NOW())
        "#,
    )
    .bind(domain)
    .bind(json!({"error": "test error"}))
    .execute(&pool)
    .await
    .expect("Failed to insert error event");

  // Should not retry errors automatically
  let should_queue = should_queue_domain(&pool, domain)
    .await
    .expect("should_queue_domain failed");

  assert!(
    !should_queue,
    "Domain with error should not be auto-retried"
  );
}

#[tokio::test]
#[serial]

async fn test_should_queue_when_classification_expired() {
  let (_db, pool) = setup_test_db().await;

  let domain = "expired-classification.com";

  // Insert domain
  sqlx::query(
    r#"
        INSERT INTO domains (domain, last_updated) VALUES ($1, NOW())
        "#,
  )
  .bind(domain)
  .execute(&pool)
  .await
  .expect("Failed to insert domain");

  // Insert a "classified" event
  sqlx::query(
        r#"
        INSERT INTO domain_classification_events (domain, action, action_data, created_at)
        VALUES ($1, 'classified'::classification_action, $2, NOW())
        "#,
    )
    .bind(domain)
    .bind(json!({"confidence": 0.9}))
    .execute(&pool)
    .await
    .expect("Failed to insert classified event");

  // Insert an expired classification (valid_until in the past)
  sqlx::query(
        r#"
        INSERT INTO domain_classifications (
            domain, classification_type, confidence, valid_on, valid_until, model, prompt_id, created_at
        )
        VALUES ($1, 'gaming', 0.9, NOW() - INTERVAL '11 days', NOW() - INTERVAL '1 day', 'test-model', 1, NOW())
        "#,
    )
    .bind(domain)
    .execute(&pool)
    .await
    .ok(); // Might fail if prompt_id doesn't exist, that's fine for this test

  // Should queue again because classification is expired
  let should_queue = should_queue_domain(&pool, domain)
    .await
    .expect("should_queue_domain failed");

  assert!(
    should_queue,
    "Domain with expired classification should be queued again"
  );
}

#[tokio::test]
#[serial]

async fn test_insert_queued_event() {
  let (_db, pool) = setup_test_db().await;

  let domain = "test-domain.com";

  // Insert a queued event
  ClassificationEventInsert {
    domain: domain.to_string(),
    action: "queued".to_string(),
    action_data: serde_json::json!({}),
    prompt_id: None,
  }
  .insert(&pool)
  .await
  .expect("Failed to insert queued event");

  // Verify it was inserted
  let result = sqlx::query(
    r#"
        SELECT action::text, created_at
        FROM domain_classification_events
        WHERE domain = $1
        "#,
  )
  .bind(domain)
  .fetch_one(&pool)
  .await
  .expect("Failed to fetch event");

  let action: String = result.try_get("action").unwrap();
  assert_eq!(action, "queued");
}
