use dns_smart_block_common::db::{
  ActiveProvisionedPattern, ClassificationSource, PromptInsert,
  ProvisionedEntry, apply_admin_classification,
  apply_admin_pattern_classification, apply_pattern_classification,
  classification_store, dns_nxdomain_classify, fetch_all_override,
  reconcile_all_provisioned, reconcile_provisioned_classifications,
  reconcile_provisioned_patterns,
};
use dns_smart_block_queue_processor::db::{
  exclude_rule_classify, insert_event,
};
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
  sqlx::query("DELETE FROM provisioned_pattern_rules")
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

// ── "all" override fetch ──────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_all_override_fetch_returns_value_when_active() {
  let (_db, pool) = setup_test_db().await;

  let domain = "always-allow.com";
  let desired = vec![ProvisionedEntry {
    domain: Some(domain.to_string()),
    pattern: None,
    classification_type: "all".to_string(),
    is_matching_site: false,
    confidence: 1.0,
    reasoning: Some("Never block — internal domain.".to_string()),
  }];

  reconcile_provisioned_classifications(&pool, &desired)
    .await
    .expect("reconcile failed");

  let result = fetch_all_override(&pool, domain)
    .await
    .expect("fetch_all_override failed");

  assert_eq!(result, Some(false));
}

#[tokio::test]
#[serial]
async fn test_all_override_fetch_returns_none_when_absent() {
  let (_db, pool) = setup_test_db().await;

  let result = fetch_all_override(&pool, "absent.com")
    .await
    .expect("fetch_all_override failed");

  assert_eq!(result, None);
}

#[tokio::test]
#[serial]
async fn test_all_override_fetch_returns_none_when_expired() {
  let (_db, pool) = setup_test_db().await;

  let domain = "expired-override.com";
  let desired = vec![ProvisionedEntry {
    domain: Some(domain.to_string()),
    pattern: None,
    classification_type: "all".to_string(),
    is_matching_site: true,
    confidence: 1.0,
    reasoning: None,
  }];

  reconcile_provisioned_classifications(&pool, &desired)
    .await
    .expect("reconcile failed");

  // Manually expire the classification.
  sqlx::query(
    "UPDATE domain_classifications \
     SET valid_until = NOW() - INTERVAL '1 day' \
     WHERE domain = $1 AND classification_type = 'all'",
  )
  .bind(domain)
  .execute(&pool)
  .await
  .unwrap();

  let result = fetch_all_override(&pool, domain)
    .await
    .expect("fetch_all_override failed");

  assert_eq!(result, None);
}

// ── exclude rule source chain ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_exclude_rule_creates_correct_source() {
  let (_db, pool) = setup_test_db().await;

  let domain = "blocked.example.com";
  let classification_type = "gaming";
  let matched_suffix = ".example.com";

  exclude_rule_classify(domain, classification_type, matched_suffix, &pool, 30)
    .await
    .expect("exclude_rule_classify failed");

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

  exclude_rule_classify(
    "a.internal.corp",
    classification_type,
    matched_suffix,
    &pool,
    30,
  )
  .await
  .unwrap();

  exclude_rule_classify(
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
    domain: Some(domain.to_string()),
    pattern: None,
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
    domain: Some("changed.com".to_string()),
    pattern: None,
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

// ── provisioned pattern rule tests ──────────────────────────────────────

fn pattern_entry(pattern: &str) -> ProvisionedEntry {
  ProvisionedEntry {
    domain: None,
    pattern: Some(pattern.to_string()),
    classification_type: "gaming".to_string(),
    is_matching_site: true,
    confidence: 1.0,
    reasoning: Some("Pattern match".to_string()),
  }
}

/// Insert a domain directly into the domains table for pattern tests.
async fn seed_domain(pool: &PgPool, domain: &str) {
  sqlx::query(
    "INSERT INTO domains (domain) VALUES ($1) ON CONFLICT DO NOTHING",
  )
  .bind(domain)
  .execute(pool)
  .await
  .unwrap();
}

/// `fetch_all_active` returns pattern rules with valid time windows.
#[tokio::test]
#[serial]
async fn test_fetch_all_active_patterns() {
  let (_db, pool) = setup_test_db().await;

  let desired = vec![pattern_entry(r"^(.*\.)?example\.com$")];
  reconcile_provisioned_patterns(&pool, &desired)
    .await
    .expect("reconcile failed");

  let active = ActiveProvisionedPattern::fetch_all_active(&pool)
    .await
    .expect("fetch_all_active failed");

  assert_eq!(active.len(), 1);
  assert_eq!(active[0].pattern, r"^(.*\.)?example\.com$");
  assert_eq!(active[0].classification_type, "gaming");
  assert!(active[0].is_matching_site);
  assert!(active[0].source_id.is_some());
}

/// `fetch_all_active` excludes expired pattern rules.
#[tokio::test]
#[serial]
async fn test_fetch_all_active_excludes_expired() {
  let (_db, pool) = setup_test_db().await;

  let desired = vec![pattern_entry(r"\.expired\.com$")];
  reconcile_provisioned_patterns(&pool, &desired)
    .await
    .unwrap();

  // Manually expire the rule.
  sqlx::query(
    "UPDATE provisioned_pattern_rules SET valid_until = NOW() - INTERVAL '1 day'",
  )
  .execute(&pool)
  .await
  .unwrap();

  let active = ActiveProvisionedPattern::fetch_all_active(&pool)
    .await
    .unwrap();
  assert!(active.is_empty(), "expired rules must not be returned");
}

/// `apply_pattern_classification` creates a classification and event for a
/// domain, using the provisioned-pattern source.
#[tokio::test]
#[serial]
async fn test_apply_pattern_classification() {
  let (_db, pool) = setup_test_db().await;

  // Create a source row for the pattern.
  let mut tx = pool.begin().await.unwrap();
  let source_id = ClassificationSource::ensure_provisioned_pattern(
    r"\.gaming\.com$",
    "gaming",
    &mut tx,
  )
  .await
  .unwrap();
  tx.commit().await.unwrap();

  let rule = ActiveProvisionedPattern {
    id: 0,
    pattern: r"\.gaming\.com$".to_string(),
    classification_type: "gaming".to_string(),
    is_matching_site: true,
    confidence: 0.99,
    reasoning: Some("Matched gaming pattern".to_string()),
    source_id: Some(source_id),
  };

  apply_pattern_classification(&pool, "sub.gaming.com", "gaming", &rule)
    .await
    .expect("apply_pattern_classification failed");

  // Classification must exist with model = "provisioned-pattern".
  let row = sqlx::query(
    "SELECT model, is_matching_site, confidence, source_id
     FROM domain_classifications
     WHERE domain = 'sub.gaming.com' AND classification_type = 'gaming'",
  )
  .fetch_one(&pool)
  .await
  .expect("classification not found");

  assert_eq!(
    row.try_get::<String, _>("model").unwrap(),
    "provisioned-pattern"
  );
  assert!(row.try_get::<bool, _>("is_matching_site").unwrap());
  assert!((row.try_get::<f32, _>("confidence").unwrap() - 0.99).abs() < 0.01);
  assert_eq!(
    row.try_get::<Option<i32>, _>("source_id").unwrap(),
    Some(source_id)
  );

  // A classified event must be written with matched_pattern in action_data.
  let event_data: serde_json::Value = sqlx::query_scalar(
    "SELECT action_data FROM domain_classification_events
     WHERE domain = 'sub.gaming.com' AND action = 'classified'",
  )
  .fetch_one(&pool)
  .await
  .expect("event not found");

  assert_eq!(event_data["matched_pattern"], r"\.gaming\.com$");
  assert_eq!(event_data["model"], "provisioned-pattern");
}

/// `apply_pattern_classification` expires existing LLM classifications for
/// the same (domain, classification_type).
#[tokio::test]
#[serial]
async fn test_apply_pattern_expires_llm_classifications() {
  let (_db, pool) = setup_test_db().await;

  let domain = "llm-then-pattern.com";

  // Create an LLM classification first.
  classification_store(
    &pool,
    domain,
    "gaming",
    true,
    0.85,
    "LLM says gaming",
    "test-model",
    "test prompt",
    "sha256:test",
    30,
  )
  .await
  .expect("classification_store failed");

  // Verify the LLM classification is active.
  let active_before: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classifications
     WHERE domain = $1 AND classification_type = 'gaming' AND valid_until > NOW()",
  )
  .bind(domain)
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(active_before, 1, "LLM classification must be active");

  // Now apply a pattern classification.
  let mut tx = pool.begin().await.unwrap();
  let source_id = ClassificationSource::ensure_provisioned_pattern(
    r"llm-then-pattern",
    "gaming",
    &mut tx,
  )
  .await
  .unwrap();
  tx.commit().await.unwrap();

  let rule = ActiveProvisionedPattern {
    id: 0,
    pattern: "llm-then-pattern".to_string(),
    classification_type: "gaming".to_string(),
    is_matching_site: true,
    confidence: 1.0,
    reasoning: Some("Pattern override".to_string()),
    source_id: Some(source_id),
  };

  apply_pattern_classification(&pool, domain, "gaming", &rule)
    .await
    .expect("apply_pattern_classification failed");

  // Only the pattern classification should be active; LLM one expired.
  let active_rows: Vec<(String,)> = sqlx::query_as(
    "SELECT model FROM domain_classifications
     WHERE domain = $1 AND classification_type = 'gaming' AND valid_until > NOW()",
  )
  .bind(domain)
  .fetch_all(&pool)
  .await
  .unwrap();

  assert_eq!(active_rows.len(), 1, "only pattern classification active");
  assert_eq!(active_rows[0].0, "provisioned-pattern");
}

/// `reconcile_provisioned_patterns` inserts new pattern rules and
/// retroactively applies them to existing matching domains.
#[tokio::test]
#[serial]
async fn test_reconcile_patterns_inserts_and_applies() {
  let (_db, pool) = setup_test_db().await;

  // Seed some domains before reconcile.
  seed_domain(&pool, "sub.gaming-site.com").await;
  seed_domain(&pool, "other.gaming-site.com").await;
  seed_domain(&pool, "unrelated.org").await;

  let desired = vec![pattern_entry(r"^(.*\.)?gaming-site\.com$")];

  let result = reconcile_provisioned_patterns(&pool, &desired)
    .await
    .expect("reconcile failed");

  assert_eq!(result.upserted, 1);
  assert_eq!(result.skipped, 0);
  assert_eq!(result.expired, 0);

  // Both matching domains should now have classifications.
  for domain in &["sub.gaming-site.com", "other.gaming-site.com"] {
    let model: String = sqlx::query_scalar(
      "SELECT model FROM domain_classifications
       WHERE domain = $1 AND classification_type = 'gaming' AND valid_until > NOW()",
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .unwrap_or_else(|_| panic!("classification missing for {domain}"));

    assert_eq!(model, "provisioned-pattern");
  }

  // The non-matching domain should have no classification.
  let unrelated_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM domain_classifications WHERE domain = 'unrelated.org'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(
    unrelated_count, 0,
    "non-matching domain must not be classified"
  );

  // Pattern rule must exist in provisioned_pattern_rules.
  let rule_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM provisioned_pattern_rules WHERE pattern = $1",
  )
  .bind(r"^(.*\.)?gaming-site\.com$")
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(rule_count, 1);
}

/// Running reconcile a second time with the same pattern skips it.
#[tokio::test]
#[serial]
async fn test_reconcile_patterns_skips_unchanged() {
  let (_db, pool) = setup_test_db().await;

  let desired = vec![pattern_entry(r"\.stable-pattern\.com$")];

  reconcile_provisioned_patterns(&pool, &desired)
    .await
    .unwrap();

  let result = reconcile_provisioned_patterns(&pool, &desired)
    .await
    .expect("second reconcile failed");

  assert_eq!(result.upserted, 0);
  assert_eq!(result.skipped, 1, "unchanged pattern must be skipped");
  assert_eq!(result.expired, 0);
}

/// Removing a pattern from the desired set expires its rule row.
#[tokio::test]
#[serial]
async fn test_reconcile_patterns_expires_removed() {
  let (_db, pool) = setup_test_db().await;

  let initial = vec![
    pattern_entry(r"\.keep\.com$"),
    pattern_entry(r"\.remove\.com$"),
  ];
  reconcile_provisioned_patterns(&pool, &initial)
    .await
    .unwrap();

  let reduced = vec![pattern_entry(r"\.keep\.com$")];
  let result = reconcile_provisioned_patterns(&pool, &reduced)
    .await
    .expect("reduce reconcile failed");

  assert_eq!(result.expired, 1, "removed pattern must be expired");
  assert_eq!(result.skipped, 1);

  // The removed pattern rule should no longer appear as active.
  let active = ActiveProvisionedPattern::fetch_all_active(&pool)
    .await
    .unwrap();
  assert_eq!(active.len(), 1);
  assert_eq!(active[0].pattern, r"\.keep\.com$");
}

/// Changing a pattern entry's values causes expire + re-insert.
#[tokio::test]
#[serial]
async fn test_reconcile_patterns_updates_changed() {
  let (_db, pool) = setup_test_db().await;

  let initial = vec![pattern_entry(r"\.change\.com$")];
  reconcile_provisioned_patterns(&pool, &initial)
    .await
    .unwrap();

  let updated = vec![ProvisionedEntry {
    domain: None,
    pattern: Some(r"\.change\.com$".to_string()),
    classification_type: "gaming".to_string(),
    is_matching_site: false, // changed
    confidence: 0.5,         // changed
    reasoning: Some("Updated".to_string()),
  }];

  let result = reconcile_provisioned_patterns(&pool, &updated)
    .await
    .expect("update reconcile failed");

  assert_eq!(result.upserted, 1, "changed pattern must be re-upserted");
  assert_eq!(result.skipped, 0);

  // Only one active rule should exist.
  let active = ActiveProvisionedPattern::fetch_all_active(&pool)
    .await
    .unwrap();
  assert_eq!(active.len(), 1);
  assert!(!active[0].is_matching_site, "updated value must apply");
  assert!((active[0].confidence - 0.5).abs() < 0.01);
}

/// `reconcile_all_provisioned` routes domain entries to domain reconcile
/// and pattern entries to pattern reconcile.
#[tokio::test]
#[serial]
async fn test_reconcile_all_mixed_entries() {
  let (_db, pool) = setup_test_db().await;

  seed_domain(&pool, "matched.example.com").await;

  let entries = vec![
    gaming_entry("exact-domain.com"),
    pattern_entry(r"^(.*\.)?example\.com$"),
  ];

  let result = reconcile_all_provisioned(&pool, &entries)
    .await
    .expect("reconcile_all failed");

  assert_eq!(
    result.upserted, 2,
    "one domain + one pattern should be upserted"
  );

  // Domain-based entry should have model = "provisioned".
  let domain_model: String = sqlx::query_scalar(
    "SELECT model FROM domain_classifications
     WHERE domain = 'exact-domain.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .expect("domain classification missing");
  assert_eq!(domain_model, "provisioned");

  // Pattern-based entry should have classified the seeded domain.
  let pattern_model: String = sqlx::query_scalar(
    "SELECT model FROM domain_classifications
     WHERE domain = 'matched.example.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .expect("pattern classification missing");
  assert_eq!(pattern_model, "provisioned-pattern");
}

/// `apply_admin_pattern_classification` creates an admin-sourced pattern rule
/// that survives a provisioned reconcile run.
#[tokio::test]
#[serial]
async fn test_admin_pattern_survives_reconcile() {
  let (_db, pool) = setup_test_db().await;

  seed_domain(&pool, "admin-matched.test.com").await;

  // Apply admin pattern (user_id = 1, the implicit admin).
  let source_id = apply_admin_pattern_classification(
    &pool,
    r"\.test\.com$",
    "gaming",
    true,
    0.95,
    "Admin-applied pattern",
    1,
  )
  .await
  .expect("apply_admin_pattern failed");

  assert!(source_id > 0);

  // The domain should be classified.
  let model: String = sqlx::query_scalar(
    "SELECT model FROM domain_classifications
     WHERE domain = 'admin-matched.test.com' AND valid_until > NOW()",
  )
  .fetch_one(&pool)
  .await
  .expect("admin pattern classification missing");
  assert_eq!(model, "provisioned-pattern");

  // Now reconcile with an empty pattern set.  Admin rules must survive.
  let result = reconcile_provisioned_patterns(&pool, &[])
    .await
    .expect("reconcile failed");

  // Admin rule uses 'admin' source type, not 'provisioned_pattern', so
  // reconcile must not touch it.
  assert_eq!(result.expired, 0, "admin pattern must not be expired");

  // The admin pattern rule should still be active.
  let rule_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM provisioned_pattern_rules
     WHERE pattern = $1 AND valid_until > NOW()",
  )
  .bind(r"\.test\.com$")
  .fetch_one(&pool)
  .await
  .unwrap();
  assert_eq!(rule_count, 1, "admin pattern rule must survive reconcile");
}

/// Provisioned pattern source rows are deduplicated by (pattern, type).
#[tokio::test]
#[serial]
async fn test_provisioned_pattern_source_deduplication() {
  let (_db, pool) = setup_test_db().await;

  let desired = vec![pattern_entry(r"\.dedup\.com$")];

  // Reconcile twice with the same pattern.
  reconcile_provisioned_patterns(&pool, &desired)
    .await
    .unwrap();

  // Change confidence to force a re-upsert, testing source dedup.
  let changed = vec![ProvisionedEntry {
    domain: None,
    pattern: Some(r"\.dedup\.com$".to_string()),
    classification_type: "gaming".to_string(),
    is_matching_site: true,
    confidence: 0.8,
    reasoning: Some("Changed".to_string()),
  }];
  reconcile_provisioned_patterns(&pool, &changed)
    .await
    .unwrap();

  // Only one source row should exist for this pattern.
  let source_count: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM classification_sources
     WHERE source_type = 'provisioned_pattern'
       AND label = $1",
  )
  .bind(r"\.dedup\.com$|gaming")
  .fetch_one(&pool)
  .await
  .unwrap();

  assert_eq!(
    source_count, 1,
    "same (pattern, type) must produce exactly one source row"
  );
}

// ── DNS NXDOMAIN classification tests ───────────────────────────────────

/// `dns_nxdomain_classify` creates a not-matching classification with
/// model "dns-nxdomain" and confidence 1.0.
#[tokio::test]
#[serial]
async fn test_nxdomain_creates_classification() {
  let (_db, pool) = setup_test_db().await;

  dns_nxdomain_classify("nonexistent.example.com", "gaming", &pool, 30)
    .await
    .expect("dns_nxdomain_classify failed");

  let row = sqlx::query(
    "SELECT model, is_matching_site, confidence, source_id
     FROM domain_classifications
     WHERE domain = 'nonexistent.example.com' AND classification_type = 'gaming'",
  )
  .fetch_one(&pool)
  .await
  .expect("classification not found");

  assert_eq!(row.try_get::<String, _>("model").unwrap(), "dns-nxdomain");
  assert!(
    !row.try_get::<bool, _>("is_matching_site").unwrap(),
    "NXDOMAIN must set is_matching_site = false"
  );
  assert!(
    (row.try_get::<f32, _>("confidence").unwrap() - 1.0).abs() < 0.01,
    "NXDOMAIN must set confidence = 1.0"
  );
  assert!(
    row
      .try_get::<Option<i32>, _>("source_id")
      .unwrap()
      .is_some(),
    "NXDOMAIN must have a source_id"
  );
}

/// `dns_nxdomain_classify` creates a classified event with NXDOMAIN reasoning.
#[tokio::test]
#[serial]
async fn test_nxdomain_creates_event() {
  let (_db, pool) = setup_test_db().await;

  dns_nxdomain_classify("gone.example.com", "gaming", &pool, 30)
    .await
    .unwrap();

  let event_data: serde_json::Value = sqlx::query_scalar(
    "SELECT action_data FROM domain_classification_events
     WHERE domain = 'gone.example.com' AND action = 'classified'",
  )
  .fetch_one(&pool)
  .await
  .expect("event not found");

  assert_eq!(event_data["model"], "dns-nxdomain");
  assert_eq!(event_data["is_matching_site"], false);
  assert_eq!(event_data["confidence"], 1.0);
  assert!(
    event_data["reasoning"]
      .as_str()
      .unwrap()
      .contains("NXDOMAIN"),
    "reasoning must mention NXDOMAIN"
  );
}

/// `dns_nxdomain_classify` uses a dns_nxdomain source type.
#[tokio::test]
#[serial]
async fn test_nxdomain_source_type() {
  let (_db, pool) = setup_test_db().await;

  dns_nxdomain_classify("nxsource.example.com", "gaming", &pool, 30)
    .await
    .unwrap();

  let source_id: i32 = sqlx::query_scalar(
    "SELECT source_id FROM domain_classifications
     WHERE domain = 'nxsource.example.com'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();

  let source_type: String = sqlx::query_scalar(
    "SELECT source_type::text FROM classification_sources WHERE id = $1",
  )
  .bind(source_id)
  .fetch_one(&pool)
  .await
  .unwrap();

  assert_eq!(source_type, "dns_nxdomain");
}

/// `dns_nxdomain_classify` respects the TTL parameter.
#[tokio::test]
#[serial]
async fn test_nxdomain_ttl() {
  let (_db, pool) = setup_test_db().await;

  dns_nxdomain_classify("ttl-test.example.com", "gaming", &pool, 7)
    .await
    .unwrap();

  let valid_until: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
    "SELECT valid_until FROM domain_classifications
     WHERE domain = 'ttl-test.example.com'",
  )
  .fetch_one(&pool)
  .await
  .unwrap();

  let days_out = (valid_until - chrono::Utc::now()).num_days();
  assert!(
    (5..=8).contains(&days_out),
    "TTL of 7 days should produce valid_until ~7 days from now, got {days_out}"
  );
}
