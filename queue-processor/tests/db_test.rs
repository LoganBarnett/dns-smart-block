use dns_smart_block_common::db::{
  ClassificationSource, ClassifierState, PromptInsert, classification_store,
};
use dns_smart_block_queue_processor::db::insert_event;
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
  sqlx::query("DELETE FROM classification_sources")
    .execute(&pool)
    .await
    .expect("Failed to clean classification_sources");
  sqlx::query("DELETE FROM prompts")
    .execute(&pool)
    .await
    .expect("Failed to clean prompts");

  (test_db, pool)
}

#[tokio::test]
#[serial]

async fn test_insert_event_classifying() {
  let (_db, pool) = setup_test_db().await;

  let domain = "test-domain.com";
  let action_data = json!({
      "model": "test-model",
      "timestamp": 1234567890
  });

  // Insert a "classifying" event
  insert_event(&pool, domain, "classifying", action_data.clone(), None)
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
#[serial]

async fn test_insert_event_classified() {
  let (_db, pool) = setup_test_db().await;

  let domain = "classified-domain.com";
  let action_data = json!({
      "is_matching_site": true,
      "confidence": 0.95,
      "model": "test-model"
  });

  // Insert a "classified" event
  insert_event(&pool, domain, "classified", action_data.clone(), None)
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
#[serial]

async fn test_insert_event_error() {
  let (_db, pool) = setup_test_db().await;

  let domain = "error-domain.com";
  let action_data = json!({
      "error_type": "OllamaApiTimeoutError",
      "error_message": "Connection timed out"
  });

  // Insert an "error" event
  insert_event(&pool, domain, "error", action_data.clone(), None)
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
#[serial]

async fn test_classification_store() {
  let (_db, pool) = setup_test_db().await;

  let domain = "gaming-site.com";
  let classification_type = "gaming";
  let confidence = 0.95;
  let model = "llama2";
  let prompt_content = "Test prompt for gaming classification";
  let prompt_hash = "sha256:abcd1234";
  let ttl_days = 10;

  // Store a classification (creates domain, prompt, source, and projection).
  let source_id = classification_store(
    &pool,
    domain,
    classification_type,
    true,
    confidence,
    "test reasoning",
    model,
    prompt_content,
    prompt_hash,
    ttl_days,
  )
  .await
  .expect("Failed to store classification");

  // Verify domain was created.
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

  // Verify prompt was created.
  let stored_content: String =
    sqlx::query_scalar(r#"SELECT content FROM prompts WHERE hash = $1"#)
      .bind(prompt_hash)
      .fetch_one(&pool)
      .await
      .expect("Failed to fetch prompt");

  assert_eq!(stored_content, prompt_content);

  // Verify classification source was created and matches the returned id.
  let cs_source_type: String = sqlx::query_scalar(
    r#"SELECT source_type::text FROM classification_sources WHERE id = $1"#,
  )
  .bind(source_id)
  .fetch_one(&pool)
  .await
  .expect("Failed to fetch classification source");

  assert_eq!(cs_source_type, "llm_prompt");

  // Verify classification projection was created with the correct source_id.
  let classification_result = sqlx::query(
    r#"
        SELECT
            domain,
            classification_type,
            confidence,
            model,
            source_id,
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
  let class_type: String = classification_result
    .try_get("classification_type")
    .unwrap();
  let class_confidence: f32 =
    classification_result.try_get("confidence").unwrap();
  let class_model: String = classification_result.try_get("model").unwrap();
  let class_source_id: i32 =
    classification_result.try_get("source_id").unwrap();
  let is_valid: bool = classification_result.try_get("is_valid").unwrap();

  assert_eq!(class_domain, domain);
  assert_eq!(class_type, classification_type);
  assert!((class_confidence - confidence as f32).abs() < 0.001);
  assert_eq!(class_model, model);
  assert_eq!(class_source_id, source_id);
  assert!(is_valid, "Classification should be valid");
}

#[tokio::test]
#[serial]

async fn test_update_projections_deduplicates_prompts() {
  let (_db, pool) = setup_test_db().await;

  let domain1 = "domain1.com";
  let domain2 = "domain2.com";
  let prompt_content = "Same prompt for both";
  let prompt_hash = "sha256:same1234";

  // Store classification for first domain.
  let source_id1 = classification_store(
    &pool,
    domain1,
    "gaming",
    true,
    0.9,
    "test reasoning 1",
    "llama2",
    prompt_content,
    prompt_hash,
    10,
  )
  .await
  .expect("Failed to store classification for domain1");

  // Store classification for second domain with the same prompt.
  let source_id2 = classification_store(
    &pool,
    domain2,
    "gaming",
    true,
    0.85,
    "test reasoning 2",
    "llama2",
    prompt_content,
    prompt_hash,
    10,
  )
  .await
  .expect("Failed to store classification for domain2");

  // Same prompt → same source record.
  assert_eq!(
    source_id1, source_id2,
    "Same prompt should produce the same source_id"
  );

  // Verify only one prompt exists.
  let prompt_count: i64 =
    sqlx::query_scalar(r#"SELECT COUNT(*) FROM prompts WHERE hash = $1"#)
      .bind(prompt_hash)
      .fetch_one(&pool)
      .await
      .expect("Failed to count prompts");

  assert_eq!(
    prompt_count, 1,
    "Should have only one prompt with the same hash"
  );

  // Verify only one classification_source exists for this prompt.
  let source_count: i64 = sqlx::query_scalar(
    r#"SELECT COUNT(*) FROM classification_sources WHERE source_type = 'llm_prompt'"#,
  )
  .fetch_one(&pool)
  .await
  .expect("Failed to count sources");

  assert_eq!(
    source_count, 1,
    "Should have one source for the shared prompt"
  );

  // Verify both classifications reference the same source_id.
  let domain1_source_id: i32 = sqlx::query_scalar(
    r#"SELECT source_id FROM domain_classifications WHERE domain = $1"#,
  )
  .bind(domain1)
  .fetch_one(&pool)
  .await
  .expect("Failed to fetch domain1 source_id");

  let domain2_source_id: i32 = sqlx::query_scalar(
    r#"SELECT source_id FROM domain_classifications WHERE domain = $1"#,
  )
  .bind(domain2)
  .fetch_one(&pool)
  .await
  .expect("Failed to fetch domain2 source_id");

  assert_eq!(domain1_source_id, source_id1);
  assert_eq!(domain2_source_id, source_id2);
}

#[tokio::test]
#[serial]

async fn test_upsert_domain_updates_timestamp() {
  let (_db, pool) = setup_test_db().await;

  let domain = "timestamp-test.com";

  // First insert
  classification_store(
    &pool,
    domain,
    "gaming",
    true,
    0.9,
    "test reasoning 1",
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
  classification_store(
    &pool,
    domain,
    "gaming",
    true,
    0.85,
    "test reasoning 2",
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

#[tokio::test]
#[serial]

async fn test_get_classifier_states_all_missing() {
  let (_db, pool) = setup_test_db().await;

  let domain = "new-domain.com";
  let classification_types =
    vec!["gaming".to_string(), "video-streaming".to_string()];

  let states =
    ClassifierState::domain_states(&pool, domain, &classification_types)
      .await
      .expect("Failed to get classifier states");

  assert_eq!(states.len(), 2);
  assert_eq!(states[0].0, "gaming");
  assert_eq!(states[0].1, ClassifierState::Missing);
  assert_eq!(states[1].0, "video-streaming");
  assert_eq!(states[1].1, ClassifierState::Missing);
}

#[tokio::test]
#[serial]

async fn test_get_classifier_states_current() {
  let (_db, pool) = setup_test_db().await;

  let domain = "current-domain.com";
  let classification_types = vec!["gaming".to_string()];

  // Create a current classification (valid for 10 days).
  classification_store(
    &pool,
    domain,
    "gaming",
    true,
    0.9,
    "test reasoning",
    "llama2",
    "test prompt",
    "sha256:test",
    10,
  )
  .await
  .expect("Failed to store classification");

  let states =
    ClassifierState::domain_states(&pool, domain, &classification_types)
      .await
      .expect("Failed to get classifier states");

  assert_eq!(states.len(), 1);
  assert_eq!(states[0].0, "gaming");
  assert_eq!(states[0].1, ClassifierState::Current);
}

/// Helper: insert a prompt and its classification_source, returning source_id.
async fn ensure_test_source(pool: &PgPool, content: &str, hash: &str) -> i32 {
  let mut tx = pool.begin().await.expect("Failed to begin tx");
  let prompt_id = PromptInsert {
    content: content.to_string(),
    hash: hash.to_string(),
  }
  .ensure(&mut tx)
  .await
  .expect("Failed to ensure prompt");
  let source_id = ClassificationSource::ensure_for_prompt(prompt_id, &mut tx)
    .await
    .expect("Failed to ensure source");
  tx.commit().await.expect("Failed to commit");
  source_id
}

#[tokio::test]
#[serial]

async fn test_get_classifier_states_expired() {
  let (_db, pool) = setup_test_db().await;

  let domain = "expired-domain.com";
  let classification_types = vec!["gaming".to_string()];

  let source_id =
    ensure_test_source(&pool, "test prompt", "sha256:expired").await;

  // Insert domain.
  sqlx::query(
    r#"
        INSERT INTO domains (domain, last_updated)
        VALUES ($1, NOW())
        "#,
  )
  .bind(domain)
  .execute(&pool)
  .await
  .expect("Failed to insert domain");

  // Insert expired classification (valid_until in the past).
  sqlx::query(
        r#"
        INSERT INTO domain_classifications
        (domain, classification_type, confidence, valid_on, valid_until, model, source_id, created_at)
        VALUES ($1, $2, $3, NOW() - INTERVAL '20 days', NOW() - INTERVAL '10 days', $4, $5, NOW() - INTERVAL '20 days')
        "#,
    )
    .bind(domain)
    .bind("gaming")
    .bind(0.9_f32)
    .bind("llama2")
    .bind(source_id)
    .execute(&pool)
    .await
    .expect("Failed to insert expired classification");

  let states =
    ClassifierState::domain_states(&pool, domain, &classification_types)
      .await
      .expect("Failed to get classifier states");

  assert_eq!(states.len(), 1);
  assert_eq!(states[0].0, "gaming");
  assert_eq!(states[0].1, ClassifierState::Expired);
}

#[tokio::test]
#[serial]

async fn test_get_classifier_states_error() {
  let (_db, pool) = setup_test_db().await;

  let domain = "error-domain.com";
  let classification_types = vec!["gaming".to_string()];

  // Insert an error event for this classification type.
  insert_event(
    &pool,
    domain,
    "error",
    json!({
        "classification_type": "gaming",
        "error": "Test error"
    }),
    None,
  )
  .await
  .expect("Failed to insert error event");

  let states =
    ClassifierState::domain_states(&pool, domain, &classification_types)
      .await
      .expect("Failed to get classifier states");

  assert_eq!(states.len(), 1);
  assert_eq!(states[0].0, "gaming");
  assert_eq!(states[0].1, ClassifierState::Error);
}

#[tokio::test]
#[serial]

async fn test_get_classifier_states_mixed() {
  let (_db, pool) = setup_test_db().await;

  let domain = "mixed-domain.com";
  let classification_types = vec![
    "gaming".to_string(),
    "video-streaming".to_string(),
    "social-media".to_string(),
    "news".to_string(),
  ];

  // gaming: Current (valid classification).
  classification_store(
    &pool,
    domain,
    "gaming",
    true,
    0.9,
    "gaming site reasoning",
    "llama2",
    "gaming prompt",
    "sha256:gaming",
    10,
  )
  .await
  .expect("Failed to store gaming classification");

  // video-streaming: Error (no classification, error event).
  insert_event(
    &pool,
    domain,
    "error",
    json!({
        "classification_type": "video-streaming",
        "error": "Timeout"
    }),
    None,
  )
  .await
  .expect("Failed to insert video-streaming error");

  // social-media: Expired (create expired classification).
  let source_id =
    ensure_test_source(&pool, "social prompt", "sha256:social").await;

  sqlx::query(
    r#"
        INSERT INTO domains (domain, last_updated)
        VALUES ($1, NOW())
        ON CONFLICT (domain) DO NOTHING
        "#,
  )
  .bind(domain)
  .execute(&pool)
  .await
  .expect("Failed to ensure domain exists");

  sqlx::query(
        r#"
        INSERT INTO domain_classifications
        (domain, classification_type, confidence, valid_on, valid_until, model, source_id, created_at)
        VALUES ($1, $2, $3, NOW() - INTERVAL '20 days', NOW() - INTERVAL '5 days', $4, $5, NOW() - INTERVAL '20 days')
        "#,
    )
    .bind(domain)
    .bind("social-media")
    .bind(0.85_f32)
    .bind("llama2")
    .bind(source_id)
    .execute(&pool)
    .await
    .expect("Failed to insert expired social-media classification");

  // news: Missing (no classification, no events).

  let states =
    ClassifierState::domain_states(&pool, domain, &classification_types)
      .await
      .expect("Failed to get classifier states");

  assert_eq!(states.len(), 4);

  // Find each state.
  let gaming_state = states.iter().find(|(name, _)| name == "gaming").unwrap();
  let video_state = states
    .iter()
    .find(|(name, _)| name == "video-streaming")
    .unwrap();
  let social_state = states
    .iter()
    .find(|(name, _)| name == "social-media")
    .unwrap();
  let news_state = states.iter().find(|(name, _)| name == "news").unwrap();

  assert_eq!(gaming_state.1, ClassifierState::Current);
  assert_eq!(video_state.1, ClassifierState::Error);
  assert_eq!(social_state.1, ClassifierState::Expired);
  assert_eq!(news_state.1, ClassifierState::Missing);
}

// Regression: prompt, classification source, and classified event must share a
// transaction so the event's source_id FK is satisfied atomically.  Previously
// insert_event was called with `pool` instead of the in-flight `&mut *tx`,
// meaning the two writes could diverge if the transaction was rolled back.
#[tokio::test]
#[serial]

async fn test_classified_event_and_source_share_transaction() {
  let (_db, pool) = setup_test_db().await;

  let domain = "atomic-test.com";
  let prompt_content = "Is this a gaming site?";
  let prompt_hash = "sha256:atomictest";

  // Simulate the queue-processor flow: open a transaction, ensure the prompt
  // and its classification source, then insert the classified event — all
  // within the same transaction.
  let mut tx = pool.begin().await.expect("Failed to begin transaction");

  let prompt_id = PromptInsert {
    content: prompt_content.to_string(),
    hash: prompt_hash.to_string(),
  }
  .ensure(&mut tx)
  .await
  .expect("Failed to ensure prompt");

  let source_id = ClassificationSource::ensure_for_prompt(prompt_id, &mut tx)
    .await
    .expect("Failed to ensure classification source");

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": "gaming",
      "is_matching_site": true,
      "confidence": 0.92,
    }),
    Some(source_id),
  )
  .await
  .expect("Failed to insert classified event");

  tx.commit().await.expect("Failed to commit transaction");

  // Prompt must be persisted with its hash.
  let stored_hash: String =
    sqlx::query_scalar("SELECT hash FROM prompts WHERE id = $1")
      .bind(prompt_id)
      .fetch_one(&pool)
      .await
      .expect("Prompt not found after commit");

  assert_eq!(stored_hash, prompt_hash, "Prompt hash should be stored");

  // Classification source must be persisted and reference the prompt.
  let cs_prompt_id: i32 = sqlx::query_scalar(
    "SELECT prompt_id FROM classification_sources WHERE id = $1",
  )
  .bind(source_id)
  .fetch_one(&pool)
  .await
  .expect("Classification source not found after commit");

  assert_eq!(
    cs_prompt_id, prompt_id,
    "Classification source should reference the prompt"
  );

  // Event must reference the correct source_id.
  let event_source_id: Option<i32> = sqlx::query_scalar(
    "SELECT source_id FROM domain_classification_events WHERE domain = $1",
  )
  .bind(domain)
  .fetch_one(&pool)
  .await
  .expect("Event not found after commit");

  assert_eq!(
    event_source_id,
    Some(source_id),
    "Classified event should reference the classification source"
  );
}

#[tokio::test]
#[serial]

async fn test_classified_event_rolls_back_with_source() {
  let (_db, pool) = setup_test_db().await;

  let domain = "rollback-test.com";
  let prompt_hash = "sha256:rollbacktest";

  let mut tx = pool.begin().await.expect("Failed to begin transaction");

  let prompt_id = PromptInsert {
    content: "Rollback test prompt".to_string(),
    hash: prompt_hash.to_string(),
  }
  .ensure(&mut tx)
  .await
  .expect("Failed to ensure prompt");

  let source_id = ClassificationSource::ensure_for_prompt(prompt_id, &mut tx)
    .await
    .expect("Failed to ensure classification source");

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({"classification_type": "gaming", "confidence": 0.9}),
    Some(source_id),
  )
  .await
  .expect("Failed to insert event within transaction");

  // Drop without committing — all three writes must be rolled back.
  drop(tx);

  let prompt_count: i64 =
    sqlx::query_scalar("SELECT COUNT(*) FROM prompts WHERE hash = $1")
      .bind(prompt_hash)
      .fetch_one(&pool)
      .await
      .expect("Failed to count prompts");

  assert_eq!(prompt_count, 0, "Rolled-back prompt must not be persisted");

  let source_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM classification_sources WHERE id = $1",
  )
  .bind(source_id)
  .fetch_one(&pool)
  .await
  .expect("Failed to count sources");

  assert_eq!(
    source_count, 0,
    "Rolled-back classification source must not be persisted"
  );

  let event_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classification_events WHERE domain = $1",
  )
  .bind(domain)
  .fetch_one(&pool)
  .await
  .expect("Failed to count events");

  assert_eq!(event_count, 0, "Rolled-back event must not be persisted");
}
