use dns_smart_block_common::db::{
  ClassificationSource, PromptInsert, ProvisionedEntry,
  apply_admin_classification, classification_store,
  reconcile_provisioned_classifications,
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

// ── provisioned classification reconcile ─────────────────────────────────

fn gaming_entry(domain: &str) -> ProvisionedEntry {
  ProvisionedEntry {
    domain: domain.to_string(),
    classification_type: "gaming".to_string(),
    is_matching_site: true,
    confidence: 0.95,
    reasoning: Some("Clear gaming content".to_string()),
  }
}

/// First reconcile inserts entries with the provisioned source type, never-
/// expiring valid_until, and writes classified events.
#[tokio::test]
#[serial]
async fn test_reconcile_inserts_new_entries() {
  let (_db, pool) = setup_test_db().await;

  let desired = vec![gaming_entry("game1.com"), gaming_entry("game2.com")];

  let result = reconcile_provisioned_classifications(&pool, &desired)
    .await
    .expect("reconcile failed");

  assert_eq!(result.upserted, 2, "both entries should be inserted");
  assert_eq!(result.skipped, 0);
  assert_eq!(result.expired, 0);

  // Exactly one provisioned source row must exist.
  let source_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM classification_sources WHERE source_type = 'provisioned'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(source_count, 1, "singleton provisioned source row");

  let source_id: i32 = sqlx::query_scalar(
    "SELECT id FROM classification_sources WHERE source_type = 'provisioned'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();

  // Each domain must have a projection referencing the provisioned source.
  for domain in &["game1.com", "game2.com"] {
    let row = sqlx::query(
      "SELECT source_id, is_matching_site, model FROM domain_classifications
       WHERE domain = $1 AND classification_type = 'gaming'",
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .unwrap_or_else(|_| panic!("projection missing for {domain}"));

    assert_eq!(
      row.try_get::<Option<i32>, _>("source_id").unwrap(),
      Some(source_id)
    );
    assert!(row.try_get::<bool, _>("is_matching_site").unwrap());
    assert_eq!(row.try_get::<String, _>("model").unwrap(), "provisioned");

    // valid_until must be far in the future (sentinel ≈ +365_000 days).
    let valid_until: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
      "SELECT valid_until FROM domain_classifications WHERE domain = $1",
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .unwrap();

    let years_out = (valid_until - chrono::Utc::now()).num_days() / 365;
    assert!(
      years_out > 900,
      "provisioned entries should never expire (got ~{years_out} years)"
    );

    // A classified event must be written.
    let event_count: i64 = sqlx::query_scalar(
      "SELECT COUNT(*) FROM domain_classification_events
       WHERE domain = $1 AND action = 'classified'",
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(event_count, 1, "one classified event per domain");
  }
}

/// Running reconcile a second time with identical data must skip all entries
/// and write no new rows or events.
#[tokio::test]
#[serial]
async fn test_reconcile_skips_unchanged_entries() {
  let (_db, pool) = setup_test_db().await;

  let desired = vec![gaming_entry("stable.com")];

  reconcile_provisioned_classifications(&pool, &desired)
    .await
    .unwrap();

  let result = reconcile_provisioned_classifications(&pool, &desired)
    .await
    .expect("second reconcile failed");

  assert_eq!(result.upserted, 0);
  assert_eq!(result.skipped, 1, "unchanged entry must be skipped");
  assert_eq!(result.expired, 0);

  // Only the first classified event should exist; no duplicate.
  let event_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classification_events
     WHERE domain = 'stable.com' AND action = 'classified'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(event_count, 1, "no duplicate events on skip");
}

/// When an entry's values change, the old projection row is expired and a
/// new one is inserted.
#[tokio::test]
#[serial]
async fn test_reconcile_updates_changed_entries() {
  let (_db, pool) = setup_test_db().await;

  let initial = vec![gaming_entry("changed.com")];
  reconcile_provisioned_classifications(&pool, &initial)
    .await
    .unwrap();

  let updated = vec![ProvisionedEntry {
    domain: "changed.com".to_string(),
    classification_type: "gaming".to_string(),
    is_matching_site: true,
    confidence: 0.55, // changed
    reasoning: Some("Updated reasoning".to_string()),
  }];

  let result = reconcile_provisioned_classifications(&pool, &updated)
    .await
    .expect("update reconcile failed");

  assert_eq!(result.upserted, 1, "changed entry must be re-upserted");
  assert_eq!(result.skipped, 0);
  assert_eq!(result.expired, 0);

  // Old projection must now be expired (valid_until <= now).
  let active_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classifications
     WHERE domain = 'changed.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(
    active_count, 1,
    "exactly one active projection after update"
  );

  // The active row must carry the new confidence.
  let confidence: f32 = sqlx::query_scalar(
    "SELECT confidence FROM domain_classifications
     WHERE domain = 'changed.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert!(
    (confidence - 0.55f32).abs() < 0.001,
    "active row must have updated confidence"
  );

  // Two classified events total (one for initial, one for update).
  let event_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classification_events
     WHERE domain = 'changed.com' AND action = 'classified'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(event_count, 2, "classified event written for each upsert");
}

/// Entries removed from the desired set must have their active projection
/// expired and an 'expired' event written.
#[tokio::test]
#[serial]
async fn test_reconcile_expires_removed_entries() {
  let (_db, pool) = setup_test_db().await;

  let initial = vec![gaming_entry("keep.com"), gaming_entry("remove.com")];
  reconcile_provisioned_classifications(&pool, &initial)
    .await
    .unwrap();

  let reduced = vec![gaming_entry("keep.com")];
  let result = reconcile_provisioned_classifications(&pool, &reduced)
    .await
    .expect("reduce reconcile failed");

  assert_eq!(result.expired, 1, "removed entry must be expired");
  assert_eq!(result.skipped, 1);
  assert_eq!(result.upserted, 0);

  // Removed domain must have no active projection.
  let active_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classifications
     WHERE domain = 'remove.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(
    active_count, 0,
    "removed domain must have no active projection"
  );

  // An 'expired' event must exist for the removed domain.
  let expired_event: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classification_events
     WHERE domain = 'remove.com' AND action = 'expired'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(
    expired_event, 1,
    "expired event must be written for removed domain"
  );

  // The kept domain must still be active.
  let keep_active: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classifications
     WHERE domain = 'keep.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(keep_active, 1, "kept domain must remain active");
}

/// Admin-sourced rows must never be expired or modified by reconcile,
/// even when the domain is absent from the desired set.
#[tokio::test]
#[serial]
async fn test_reconcile_leaves_admin_rows_untouched() {
  let (_db, pool) = setup_test_db().await;

  // user_id = 1 (the implicit admin) is seeded by the initial migration.

  // Write an admin classification for a domain that won't appear in desired.
  apply_admin_classification(
    &pool,
    "manual.com",
    "gaming",
    true,
    1.0,
    "Manually classified",
    1,
    None, // never expires
  )
  .await
  .expect("apply_admin_classification failed");

  // Reconcile with a completely different domain.
  let desired = vec![gaming_entry("provisioned-only.com")];
  let result = reconcile_provisioned_classifications(&pool, &desired)
    .await
    .expect("reconcile failed");

  assert_eq!(result.expired, 0, "reconcile must not expire admin rows");

  // Admin classification must still be active.
  let admin_active: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classifications
     WHERE domain = 'manual.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(
    admin_active, 1,
    "admin classification must survive reconcile"
  );
}

/// Multiple reconcile runs must share a single provisioned source row.
#[tokio::test]
#[serial]
async fn test_reconcile_singleton_source_row() {
  let (_db, pool) = setup_test_db().await;

  reconcile_provisioned_classifications(&pool, &[gaming_entry("a.com")])
    .await
    .unwrap();
  reconcile_provisioned_classifications(&pool, &[gaming_entry("b.com")])
    .await
    .unwrap();
  reconcile_provisioned_classifications(&pool, &[gaming_entry("c.com")])
    .await
    .unwrap();

  let source_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM classification_sources WHERE source_type = 'provisioned'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();

  assert_eq!(
    source_count, 1,
    "all reconcile runs must share one provisioned source row"
  );
}
