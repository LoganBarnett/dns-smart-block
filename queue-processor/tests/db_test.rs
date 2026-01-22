use dns_smart_block_queue_processor::db::{insert_event, update_projections};
use serde_json::json;
use sqlx::{PgPool, Row};

/// Helper to set up a test database
/// Note: This requires DATABASE_URL to be set to a test database
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

    sqlx::query("DELETE FROM prompts")
        .execute(&pool)
        .await
        .expect("Failed to clean prompts");

    pool
}

#[tokio::test]
#[ignore] // Requires DATABASE_URL
async fn test_insert_event_classifying() {
    let pool = setup_test_db().await;

    let domain = "test-domain.com";
    let action_data = json!({
        "model": "test-model",
        "timestamp": 1234567890
    });

    // Insert a "classifying" event
    insert_event(&pool, domain, "classifying", action_data.clone())
        .await
        .expect("Failed to insert classifying event");

    // Verify it was inserted
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
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch event");

    let action: String = result.try_get("action").unwrap();
    let data: serde_json::Value = result.try_get("action_data").unwrap();

    assert_eq!(action, "classifying");
    assert_eq!(data["model"], "test-model");
}

#[tokio::test]
#[ignore] // Requires DATABASE_URL
async fn test_insert_event_classified() {
    let pool = setup_test_db().await;

    let domain = "classified-domain.com";
    let action_data = json!({
        "is_matching_site": true,
        "confidence": 0.95,
        "model": "test-model"
    });

    // Insert a "classified" event
    insert_event(&pool, domain, "classified", action_data.clone())
        .await
        .expect("Failed to insert classified event");

    // Verify it was inserted
    let result = sqlx::query(
        r#"
        SELECT action::text, action_data
        FROM domain_classification_events
        WHERE domain = $1
        "#,
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch event");

    let action: String = result.try_get("action").unwrap();
    let data: serde_json::Value = result.try_get("action_data").unwrap();

    assert_eq!(action, "classified");
    assert_eq!(data["confidence"], 0.95);
    assert_eq!(data["is_matching_site"], true);
}

#[tokio::test]
#[ignore] // Requires DATABASE_URL
async fn test_insert_event_error() {
    let pool = setup_test_db().await;

    let domain = "error-domain.com";
    let action_data = json!({
        "error_type": "OllamaApiTimeoutError",
        "error_message": "Connection timed out"
    });

    // Insert an "error" event
    insert_event(&pool, domain, "error", action_data.clone())
        .await
        .expect("Failed to insert error event");

    // Verify it was inserted
    let result = sqlx::query(
        r#"
        SELECT action::text, action_data
        FROM domain_classification_events
        WHERE domain = $1
        "#,
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch event");

    let action: String = result.try_get("action").unwrap();
    let data: serde_json::Value = result.try_get("action_data").unwrap();

    assert_eq!(action, "error");
    assert_eq!(data["error_type"], "OllamaApiTimeoutError");
}

#[tokio::test]
#[ignore] // Requires DATABASE_URL
async fn test_update_projections() {
    let pool = setup_test_db().await;

    let domain = "gaming-site.com";
    let classification_type = "gaming";
    let confidence = 0.95;
    let model = "llama2";
    let prompt_content = "Test prompt for gaming classification";
    let prompt_hash = "sha256:abcd1234";
    let ttl_days = 10;

    // Update projections (creates domain, prompt, and classification)
    update_projections(
        &pool,
        domain,
        classification_type,
        confidence,
        model,
        prompt_content,
        prompt_hash,
        ttl_days,
    )
    .await
    .expect("Failed to update projections");

    // Verify domain was created
    let domain_result = sqlx::query(
        r#"
        SELECT domain, last_updated FROM domains WHERE domain = $1
        "#,
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch domain");

    let found_domain: String = domain_result.try_get("domain").unwrap();
    assert_eq!(found_domain, domain);

    // Verify prompt was created
    let prompt_result = sqlx::query(
        r#"
        SELECT id, content, hash FROM prompts WHERE hash = $1
        "#,
    )
    .bind(prompt_hash)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch prompt");

    let prompt_id: i32 = prompt_result.try_get("id").unwrap();
    let stored_content: String = prompt_result.try_get("content").unwrap();
    assert_eq!(stored_content, prompt_content);

    // Verify classification was created
    let classification_result = sqlx::query(
        r#"
        SELECT
            domain,
            classification_type,
            confidence,
            model,
            prompt_id,
            valid_until > NOW() as is_valid
        FROM domain_classifications
        WHERE domain = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch classification");

    let class_domain: String = classification_result.try_get("domain").unwrap();
    let class_type: String = classification_result.try_get("classification_type").unwrap();
    let class_confidence: f32 = classification_result.try_get("confidence").unwrap();
    let class_model: String = classification_result.try_get("model").unwrap();
    let class_prompt_id: i32 = classification_result.try_get("prompt_id").unwrap();
    let is_valid: bool = classification_result.try_get("is_valid").unwrap();

    assert_eq!(class_domain, domain);
    assert_eq!(class_type, classification_type);
    assert!((class_confidence - confidence as f32).abs() < 0.001);
    assert_eq!(class_model, model);
    assert_eq!(class_prompt_id, prompt_id);
    assert!(is_valid, "Classification should be valid");
}

#[tokio::test]
#[ignore] // Requires DATABASE_URL
async fn test_update_projections_deduplicates_prompts() {
    let pool = setup_test_db().await;

    let domain1 = "domain1.com";
    let domain2 = "domain2.com";
    let prompt_content = "Same prompt for both";
    let prompt_hash = "sha256:same1234";

    // Update projections for first domain
    update_projections(
        &pool,
        domain1,
        "gaming",
        0.9,
        "llama2",
        prompt_content,
        prompt_hash,
        10,
    )
    .await
    .expect("Failed to update projections for domain1");

    // Update projections for second domain with same prompt
    update_projections(
        &pool,
        domain2,
        "gaming",
        0.85,
        "llama2",
        prompt_content,
        prompt_hash,
        10,
    )
    .await
    .expect("Failed to update projections for domain2");

    // Verify only one prompt exists
    let prompt_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM prompts WHERE hash = $1
        "#,
    )
    .bind(prompt_hash)
    .fetch_one(&pool)
    .await
    .expect("Failed to count prompts");

    assert_eq!(prompt_count, 1, "Should have only one prompt with the same hash");

    // Verify both classifications reference the same prompt
    let prompt_id: i32 = sqlx::query_scalar(
        r#"
        SELECT id FROM prompts WHERE hash = $1
        "#,
    )
    .bind(prompt_hash)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch prompt id");

    let domain1_prompt_id: i32 = sqlx::query_scalar(
        r#"
        SELECT prompt_id FROM domain_classifications WHERE domain = $1
        "#,
    )
    .bind(domain1)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch domain1 prompt_id");

    let domain2_prompt_id: i32 = sqlx::query_scalar(
        r#"
        SELECT prompt_id FROM domain_classifications WHERE domain = $1
        "#,
    )
    .bind(domain2)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch domain2 prompt_id");

    assert_eq!(domain1_prompt_id, prompt_id);
    assert_eq!(domain2_prompt_id, prompt_id);
}

#[tokio::test]
#[ignore] // Requires DATABASE_URL
async fn test_upsert_domain_updates_timestamp() {
    let pool = setup_test_db().await;

    let domain = "timestamp-test.com";

    // First insert
    update_projections(
        &pool,
        domain,
        "gaming",
        0.9,
        "llama2",
        "test prompt",
        "sha256:test1",
        10,
    )
    .await
    .expect("Failed first insert");

    let first_timestamp: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        r#"
        SELECT last_updated FROM domains WHERE domain = $1
        "#,
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch first timestamp");

    // Wait a moment
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Second insert with different classification
    update_projections(
        &pool,
        domain,
        "gaming",
        0.85,
        "llama2",
        "test prompt 2",
        "sha256:test2",
        10,
    )
    .await
    .expect("Failed second insert");

    let second_timestamp: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        r#"
        SELECT last_updated FROM domains WHERE domain = $1
        "#,
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch second timestamp");

    assert!(
        second_timestamp > first_timestamp,
        "Second timestamp should be later than first"
    );

    // Verify we have 2 classifications for the same domain
    let classification_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM domain_classifications WHERE domain = $1
        "#,
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .expect("Failed to count classifications");

    assert_eq!(classification_count, 2, "Should have 2 classifications");
}
