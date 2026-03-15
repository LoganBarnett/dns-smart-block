use dns_smart_block_common::db::{
  ClassificationSource, PromptInsert, classification_store,
};
use dns_smart_block_queue_processor::db::{apply_exclude_rule, insert_event};
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
    .unwrap();
  sqlx::query("DELETE FROM domain_classifications")
    .execute(&pool)
    .await
    .unwrap();
  sqlx::query("DELETE FROM domains")
    .execute(&pool)
    .await
    .unwrap();
  sqlx::query("DELETE FROM classification_sources")
    .execute(&pool)
    .await
    .unwrap();
  sqlx::query("DELETE FROM prompts")
    .execute(&pool)
    .await
    .unwrap();

  (test_db, pool)
}

// ── exclude rule source chain ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_exclude_rule_creates_correct_source() {
  let (_db, pool) = setup_test_db().await;

  let domain = "blocked.example.com";
  let classification_type = "gaming";
  let matched_suffix = ".example.com";

  apply_exclude_rule(domain, classification_type, matched_suffix, &pool, 30)
    .await
    .expect("apply_exclude_rule failed");

  // Source row must be config_exclude_rule with label = matched suffix.
  let source_row = sqlx::query(
    "SELECT source_type::text, label FROM classification_sources WHERE source_type = 'config_exclude_rule'",
  )
  .fetch_one(&pool)
  .await
  .expect("classification_source not found");

  assert_eq!(
    source_row.try_get::<String, _>("source_type").unwrap(),
    "config_exclude_rule"
  );
  assert_eq!(
    source_row.try_get::<String, _>("label").unwrap(),
    matched_suffix
  );

  let source_id: i32 = sqlx::query_scalar(
    "SELECT id FROM classification_sources WHERE source_type = 'config_exclude_rule' AND label = $1",
  )
  .bind(matched_suffix)
  .fetch_one(&pool)
  .await
  .unwrap();

  // Classification projection must reference the exclude-rule source.
  let class_source_id: i32 = sqlx::query_scalar(
    "SELECT source_id FROM domain_classifications WHERE domain = $1 AND classification_type = $2",
  )
  .bind(domain)
  .bind(classification_type)
  .fetch_one(&pool)
  .await
  .expect("classification not found");

  assert_eq!(
    class_source_id, source_id,
    "classification.source_id must reference the exclude-rule source"
  );

  // Audit event must reference the same source.
  let event_source_id: Option<i32> = sqlx::query_scalar(
    "SELECT source_id FROM domain_classification_events WHERE domain = $1",
  )
  .bind(domain)
  .fetch_one(&pool)
  .await
  .expect("classified event not found");

  assert_eq!(
    event_source_id,
    Some(source_id),
    "classified event.source_id must reference the exclude-rule source"
  );

  // Excluded domains must never be flagged as matching.
  let is_matching: bool = sqlx::query_scalar(
    "SELECT is_matching_site FROM domain_classifications WHERE domain = $1 AND classification_type = $2",
  )
  .bind(domain)
  .bind(classification_type)
  .fetch_one(&pool)
  .await
  .unwrap();

  assert!(!is_matching, "excluded domain must not be is_matching_site");
}

#[tokio::test]
#[serial]
async fn test_exclude_rule_source_is_deduplicated() {
  let (_db, pool) = setup_test_db().await;

  let matched_suffix = ".internal.corp";
  let classification_type = "gaming";

  apply_exclude_rule(
    "a.internal.corp",
    classification_type,
    matched_suffix,
    &pool,
    30,
  )
  .await
  .unwrap();

  apply_exclude_rule(
    "b.internal.corp",
    classification_type,
    matched_suffix,
    &pool,
    30,
  )
  .await
  .unwrap();

  // Only one source row should exist for the same pattern.
  let source_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM classification_sources WHERE source_type = 'config_exclude_rule' AND label = $1",
  )
  .bind(matched_suffix)
  .fetch_one(&pool)
  .await
  .unwrap();

  assert_eq!(
    source_count, 1,
    "same exclude suffix must produce exactly one source record"
  );

  let source_id: i32 = sqlx::query_scalar(
    "SELECT id FROM classification_sources WHERE source_type = 'config_exclude_rule' AND label = $1",
  )
  .bind(matched_suffix)
  .fetch_one(&pool)
  .await
  .unwrap();

  let a_source: i32 = sqlx::query_scalar(
    "SELECT source_id FROM domain_classifications WHERE domain = 'a.internal.corp'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();

  let b_source: i32 = sqlx::query_scalar(
    "SELECT source_id FROM domain_classifications WHERE domain = 'b.internal.corp'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();

  assert_eq!(
    a_source, source_id,
    "first domain must reference the shared source"
  );
  assert_eq!(
    b_source, source_id,
    "second domain must reference the shared source"
  );
}

// ── LLM classification source chain ──────────────────────────────────────

/// Verify that classification_store produces an llm_prompt source and that
/// an event written inside the same transaction references that source_id.
#[tokio::test]
#[serial]
async fn test_llm_classification_source_chain() {
  let (_db, pool) = setup_test_db().await;

  let domain = "llm-classified.com";
  let prompt_content = "Is this site about gaming?";
  let prompt_hash = "sha256:llmtest1";

  // classification_store creates prompt → source → projection atomically.
  let source_id = classification_store(
    &pool,
    domain,
    "gaming",
    true,
    0.91,
    "Strong gaming signals in page content",
    "llama2",
    prompt_content,
    prompt_hash,
    10,
  )
  .await
  .expect("classification_store failed");

  // Source must be llm_prompt with a prompt_id.
  let source_row = sqlx::query(
    "SELECT source_type::text, prompt_id FROM classification_sources WHERE id = $1",
  )
  .bind(source_id)
  .fetch_one(&pool)
  .await
  .expect("classification_source not found");

  assert_eq!(
    source_row.try_get::<String, _>("source_type").unwrap(),
    "llm_prompt"
  );
  assert!(
    source_row
      .try_get::<Option<i32>, _>("prompt_id")
      .unwrap()
      .is_some(),
    "llm_prompt source must have a prompt_id"
  );

  // Now simulate the queue-processor flow: open a transaction, ensure the
  // same prompt and source, then write the classified event.
  let mut tx = pool.begin().await.unwrap();

  let prompt_id = PromptInsert {
    content: prompt_content.to_string(),
    hash: prompt_hash.to_string(),
  }
  .ensure(&mut tx)
  .await
  .unwrap();

  let event_source_id =
    ClassificationSource::ensure_for_prompt(prompt_id, &mut tx)
      .await
      .unwrap();

  // Same prompt → same source_id (deduplication).
  assert_eq!(
    event_source_id, source_id,
    "re-ensuring the same prompt must return the same source_id"
  );

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": "gaming",
      "is_matching_site": true,
      "confidence": 0.91,
    }),
    Some(event_source_id),
  )
  .await
  .unwrap();

  tx.commit().await.unwrap();

  // Event must reference the llm_prompt source.
  let ev_source: Option<i32> = sqlx::query_scalar(
    "SELECT source_id FROM domain_classification_events WHERE domain = $1",
  )
  .bind(domain)
  .fetch_one(&pool)
  .await
  .unwrap();

  assert_eq!(
    ev_source,
    Some(source_id),
    "classified event must carry the llm_prompt source_id"
  );
}
