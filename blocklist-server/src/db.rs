pub use dns_smart_block_common::db::*;

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::{Duration, Utc};
  use serial_test::serial;
  use sqlx::{PgPool, Row};

  async fn setup_test_db() -> (dns_smart_block_common::test_db::TestDb, PgPool)
  {
    let test_db = dns_smart_block_common::test_db::TestDb::new()
      .expect("failed to start test db");
    let pool = test_db.pool().await.expect("failed to get pool");

    sqlx::query("DELETE FROM domain_classification_events")
      .execute(&pool)
      .await
      .expect("Failed to clean events");
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

  /// Insert a prompt and its classification_source, returning source_id.
  async fn ensure_test_source(pool: &PgPool, content: &str, hash: &str) -> i32 {
    sqlx::query(
      "INSERT INTO prompts (content, hash, created_at) VALUES ($1, $2, NOW()) ON CONFLICT (hash) DO NOTHING",
    )
    .bind(content)
    .bind(hash)
    .execute(pool)
    .await
    .unwrap();

    let prompt_id: i32 =
      sqlx::query_scalar("SELECT id FROM prompts WHERE hash = $1")
        .bind(hash)
        .fetch_one(pool)
        .await
        .unwrap();

    sqlx::query_scalar::<_, i32>(
      r#"
      INSERT INTO classification_sources (source_type, prompt_id, created_at)
      VALUES ('llm_prompt', $1, NOW())
      ON CONFLICT (prompt_id) WHERE prompt_id IS NOT NULL DO NOTHING
      RETURNING 0
      "#,
    )
    .bind(prompt_id)
    .fetch_optional(pool)
    .await
    .unwrap();

    sqlx::query_scalar(
      "SELECT id FROM classification_sources WHERE prompt_id = $1 AND source_type = 'llm_prompt'",
    )
    .bind(prompt_id)
    .fetch_one(pool)
    .await
    .unwrap()
  }

  #[tokio::test]
  #[serial]

  async fn test_get_blocked_domains_at_current_time() {
    let (_db, pool) = setup_test_db().await;

    let source_id =
      ensure_test_source(&pool, "test prompt", "sha256:test").await;

    // Insert test domains
    let domains = vec!["gaming1.com", "gaming2.com", "news1.com"];
    for domain in &domains {
      sqlx::query(
        "INSERT INTO domains (domain, last_updated) VALUES ($1, NOW())",
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
            model, source_id, created_at
        )
        VALUES ($1, 'gaming', 0.9, $2, $3, 'test-model', $4, NOW())
        "#,
      )
      .bind(domain)
      .bind(now)
      .bind(valid_until)
      .bind(source_id)
      .execute(&pool)
      .await
      .unwrap();
    }

    // Insert news classification
    sqlx::query(
      r#"
      INSERT INTO domain_classifications (
          domain, classification_type, confidence, valid_on, valid_until,
          model, source_id, created_at
      )
      VALUES ($1, 'news', 0.95, $2, $3, 'test-model', $4, NOW())
      "#,
    )
    .bind("news1.com")
    .bind(now)
    .bind(valid_until)
    .bind(source_id)
    .execute(&pool)
    .await
    .unwrap();

    // Query gaming domains
    let gaming_domains =
      get_blocked_domains(&pool, "gaming", None).await.unwrap();
    assert_eq!(gaming_domains.len(), 2);
    assert!(gaming_domains.contains(&"gaming1.com".to_string()));
    assert!(gaming_domains.contains(&"gaming2.com".to_string()));

    // Query news domains
    let news_domains = get_blocked_domains(&pool, "news", None).await.unwrap();
    assert_eq!(news_domains.len(), 1);
    assert!(news_domains.contains(&"news1.com".to_string()));
  }

  #[tokio::test]
  #[serial]

  async fn test_get_blocked_domains_excludes_expired() {
    let (_db, pool) = setup_test_db().await;

    let source_id =
      ensure_test_source(&pool, "test prompt", "sha256:test").await;

    sqlx::query(
      "INSERT INTO domains (domain, last_updated) VALUES ('expired.com', NOW())",
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
          model, source_id, created_at
      )
      VALUES ($1, 'gaming', 0.9, $2, $3, 'test-model', $4, NOW())
      "#,
    )
    .bind("expired.com")
    .bind(expired - Duration::days(10))
    .bind(expired)
    .bind(source_id)
    .execute(&pool)
    .await
    .unwrap();

    // Should not return expired domain
    let domains = get_blocked_domains(&pool, "gaming", None).await.unwrap();
    assert_eq!(domains.len(), 0);
  }

  #[tokio::test]
  #[serial]

  async fn test_get_blocked_domains_at_specific_time() {
    let (_db, pool) = setup_test_db().await;

    let source_id =
      ensure_test_source(&pool, "test prompt", "sha256:test").await;

    sqlx::query(
      "INSERT INTO domains (domain, last_updated) VALUES ('future.com', NOW())",
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
          model, source_id, created_at
      )
      VALUES ($1, 'gaming', 0.9, $2, $3, 'test-model', $4, NOW())
      "#,
    )
    .bind("future.com")
    .bind(future_start)
    .bind(future_end)
    .bind(source_id)
    .execute(&pool)
    .await
    .unwrap();

    // Should not return at current time
    let domains_now = get_blocked_domains(&pool, "gaming", None).await.unwrap();
    assert_eq!(domains_now.len(), 0);

    // Should return at future time
    let domains_future = get_blocked_domains(
      &pool,
      "gaming",
      Some(future_start + Duration::hours(1)),
    )
    .await
    .unwrap();
    assert_eq!(domains_future.len(), 1);
    assert!(domains_future.contains(&"future.com".to_string()));
  }

  #[tokio::test]
  #[serial]
  async fn test_rebuild_projections_from_events() {
    let test_db = dns_smart_block_common::test_db::TestDb::new()
      .expect("Failed to setup test database");
    let pool = test_db.pool().await.expect("Failed to get pool");

    // Clean up test data (in correct order for foreign key constraints)
    sqlx::query("DELETE FROM domain_classification_events")
      .execute(&pool)
      .await
      .expect("Failed to clean events");
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

    // Insert test prompts and their classification sources.
    let source1_id =
      ensure_test_source(&pool, "test prompt 1", "sha256:test1").await;
    let source2_id =
      ensure_test_source(&pool, "test prompt 2", "sha256:test2").await;

    // Insert older classified event WITHOUT model field (simulating legacy event)
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, source_id, created_at
            )
            VALUES (
                'old-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": true, "confidence": 0.9, "reasoning": "Old event without model"}',
                $1,
                NOW() - INTERVAL '2 days'
            )
            "#,
    )
    .bind(source1_id)
    .execute(&pool)
    .await
    .unwrap();

    // Insert newer classified event WITH model field
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, source_id, created_at
            )
            VALUES (
                'new-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": true, "confidence": 0.85, "reasoning": "New event with model", "model": "test-model-v1"}',
                $1,
                NOW() - INTERVAL '1 day'
            )
            "#,
    )
    .bind(source2_id)
    .execute(&pool)
    .await
    .unwrap();

    // Insert negative classification event
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, source_id, created_at
            )
            VALUES (
                'non-gaming.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": false, "confidence": 0.95, "reasoning": "Not a gaming site", "model": "test-model-v1"}',
                $1,
                NOW() - INTERVAL '1 day'
            )
            "#,
    )
    .bind(source2_id)
    .execute(&pool)
    .await
    .unwrap();

    // Insert multiple events for same domain (should take latest)
    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, source_id, created_at
            )
            VALUES (
                'updated-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": true, "confidence": 0.7, "reasoning": "First classification", "model": "test-model-v1"}',
                $1,
                NOW() - INTERVAL '3 days'
            )
            "#,
    )
    .bind(source1_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
      r#"
            INSERT INTO domain_classification_events (
                domain, action, action_data, source_id, created_at
            )
            VALUES (
                'updated-domain.com',
                'classified',
                '{"classification_type": "gaming", "is_matching_site": false, "confidence": 0.92, "reasoning": "Updated classification", "model": "test-model-v2"}',
                $1,
                NOW() - INTERVAL '1 day'
            )
            "#,
    )
    .bind(source2_id)
    .execute(&pool)
    .await
    .unwrap();

    // Rebuild projections
    let count = rebuild_projections_from_events(&pool, 7).await.unwrap();
    assert_eq!(count, 4); // Should create 4 projections

    // Verify old event without model got 'unknown' as model
    let old_classification: (String, bool, f32, String) = sqlx::query_as(
      r#"
            SELECT model, is_matching_site, confidence, reasoning
            FROM domain_classifications
            WHERE domain = 'old-domain.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(old_classification.0, "unknown");
    assert_eq!(old_classification.1, true);
    assert_eq!(old_classification.2, 0.9);
    assert_eq!(old_classification.3, "Old event without model");

    // Verify new event with model preserved the model value
    let new_classification: (String, bool, f32, String) = sqlx::query_as(
      r#"
            SELECT model, is_matching_site, confidence, reasoning
            FROM domain_classifications
            WHERE domain = 'new-domain.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(new_classification.0, "test-model-v1");
    assert_eq!(new_classification.1, true);
    assert_eq!(new_classification.2, 0.85);
    assert_eq!(new_classification.3, "New event with model");

    // Verify negative classification was created
    let negative_classification: (bool, f32) = sqlx::query_as(
      r#"
            SELECT is_matching_site, confidence
            FROM domain_classifications
            WHERE domain = 'non-gaming.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(negative_classification.0, false);
    assert_eq!(negative_classification.1, 0.95);

    // Verify updated domain used latest event
    let updated_classification: (bool, f32, String, String) = sqlx::query_as(
      r#"
            SELECT is_matching_site, confidence, reasoning, model
            FROM domain_classifications
            WHERE domain = 'updated-domain.com' AND classification_type = 'gaming'
            "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(updated_classification.0, false); // Latest is negative
    assert_eq!(updated_classification.1, 0.92);
    assert_eq!(updated_classification.2, "Updated classification");
    assert_eq!(updated_classification.3, "test-model-v2");

    // Verify domains table was populated
    let domain_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domains")
      .fetch_one(&pool)
      .await
      .unwrap();
    assert_eq!(domain_count, 4);
  }

  // Helper to insert an event with an explicit timestamp offset from NOW().
  async fn insert_event_at(
    pool: &PgPool,
    domain: &str,
    action: &str,
    seconds_ago: i64,
  ) {
    sqlx::query(
      r#"
      INSERT INTO domain_classification_events (domain, action, action_data, created_at)
      VALUES ($1, $2::classification_action, '{}'::jsonb, NOW() - ($3 * INTERVAL '1 second'))
      "#,
    )
    .bind(domain)
    .bind(action)
    .bind(seconds_ago)
    .execute(pool)
    .await
    .unwrap();
  }

  #[tokio::test]
  #[serial]
  async fn test_events_by_action_counts_latest_state_only() {
    let (_db, pool) = setup_test_db().await;

    // domain-a: classifying → error → classifying → classified
    // Only "classified" (the latest) should be counted.
    insert_event_at(&pool, "domain-a.com", "classifying", 30).await;
    insert_event_at(&pool, "domain-a.com", "error", 20).await;
    insert_event_at(&pool, "domain-a.com", "classifying", 10).await;
    insert_event_at(&pool, "domain-a.com", "classified", 5).await;

    let stats = get_metrics_stats(&pool).await.unwrap();

    assert_eq!(
      stats
        .events_by_action
        .get("classified")
        .copied()
        .unwrap_or(0),
      1
    );
    assert_eq!(
      stats
        .events_by_action
        .get("classifying")
        .copied()
        .unwrap_or(0),
      0
    );
    assert_eq!(stats.events_by_action.get("error").copied().unwrap_or(0), 0);
  }

  #[tokio::test]
  #[serial]
  async fn test_events_by_action_multiple_domains_mixed_states() {
    let (_db, pool) = setup_test_db().await;

    // domain-a: ended up classified
    insert_event_at(&pool, "domain-a.com", "classifying", 20).await;
    insert_event_at(&pool, "domain-a.com", "classified", 10).await;

    // domain-b: ended up in error
    insert_event_at(&pool, "domain-b.com", "classifying", 20).await;
    insert_event_at(&pool, "domain-b.com", "error", 10).await;

    // domain-c: still classifying
    insert_event_at(&pool, "domain-c.com", "classifying", 5).await;

    let stats = get_metrics_stats(&pool).await.unwrap();

    assert_eq!(
      stats
        .events_by_action
        .get("classified")
        .copied()
        .unwrap_or(0),
      1
    );
    assert_eq!(stats.events_by_action.get("error").copied().unwrap_or(0), 1);
    assert_eq!(
      stats
        .events_by_action
        .get("classifying")
        .copied()
        .unwrap_or(0),
      1
    );
    // "queued" is no longer written anywhere; should never appear
    assert_eq!(
      stats.events_by_action.get("queued").copied().unwrap_or(0),
      0
    );
  }

  #[tokio::test]
  #[serial]
  async fn test_events_by_action_empty_table() {
    let (_db, pool) = setup_test_db().await;

    let stats = get_metrics_stats(&pool).await.unwrap();

    assert!(stats.events_by_action.is_empty());
  }

  // ── get_domain_status tests ───────────────────────────────────────────────

  #[tokio::test]
  #[serial]
  async fn test_get_domain_status_unknown_domain() {
    let (_db, pool) = setup_test_db().await;

    let status = get_domain_status(&pool, "unknown.com")
      .await
      .expect("get_domain_status failed");

    assert!(!status.exists, "unknown domain should not exist");
    assert!(status.classifications.is_empty());
    assert!(status.recent_events.is_empty());
  }

  #[tokio::test]
  #[serial]
  async fn test_get_domain_status_with_llm_classification() {
    let (_db, pool) = setup_test_db().await;

    let source_id =
      ensure_test_source(&pool, "Is this gaming?", "sha256:gaming-prompt")
        .await;

    sqlx::query(
      "INSERT INTO domains (domain, last_updated) VALUES ($1, NOW())",
    )
    .bind("gaming-site.com")
    .execute(&pool)
    .await
    .unwrap();

    let now = Utc::now();
    sqlx::query(
      r#"
      INSERT INTO domain_classifications
        (domain, classification_type, is_matching_site, confidence, reasoning,
         valid_on, valid_until, model, source_id, created_at)
      VALUES ($1, 'gaming', true, 0.92, 'Lots of game content', $2, $3, 'llama2', $4, NOW())
      "#,
    )
    .bind("gaming-site.com")
    .bind(now)
    .bind(now + Duration::days(10))
    .bind(source_id)
    .execute(&pool)
    .await
    .unwrap();

    let status = get_domain_status(&pool, "gaming-site.com")
      .await
      .expect("get_domain_status failed");

    assert!(status.exists);
    assert_eq!(status.classifications.len(), 1);
    let c = &status.classifications[0];
    assert_eq!(c.classification_type, "gaming");
    assert!(c.is_matching_site);
    assert_eq!(
      c.source_type.as_deref(),
      Some("llm_prompt"),
      "LLM classification should have llm_prompt source_type"
    );
    assert!(c.source_label.is_none(), "llm_prompt sources have no label");
  }

  #[tokio::test]
  #[serial]
  async fn test_get_domain_status_with_admin_classification() {
    let (_db, pool) = setup_test_db().await;

    // Seed admin user id=1 if not present (users table is not cleaned between tests).
    let admin_id: i32 =
      sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'admin'")
        .fetch_one(&pool)
        .await
        .expect("admin user not found — seed migration may be missing");

    let source_id = apply_admin_classification(
      &pool,
      "override.com",
      "gaming",
      false,
      1.0,
      "Manually reviewed: not a gaming site",
      admin_id,
      Some(30),
    )
    .await
    .expect("apply_admin_classification failed");

    // Verify source record.
    let row = sqlx::query(
      "SELECT source_type::text, user_id FROM classification_sources WHERE id = $1",
    )
    .bind(source_id)
    .fetch_one(&pool)
    .await
    .expect("classification_source not found");

    let source_type: String = row.try_get("source_type").unwrap();
    let user_id: i32 = row.try_get("user_id").unwrap();
    assert_eq!(source_type, "admin");
    assert_eq!(user_id, admin_id);

    // Verify classification projection references the source.
    let status = get_domain_status(&pool, "override.com")
      .await
      .expect("get_domain_status failed");

    assert!(status.exists);
    assert_eq!(status.classifications.len(), 1);
    let c = &status.classifications[0];
    assert_eq!(c.source_type.as_deref(), Some("admin"));
    assert_eq!(c.source_user_id, Some(admin_id));
    assert!(!c.is_matching_site);

    // Verify event references the same source.
    assert_eq!(status.recent_events.len(), 1);
    let e = &status.recent_events[0];
    assert_eq!(e.action, "classified");
    assert_eq!(
      e.source_type.as_deref(),
      Some("admin"),
      "classified event should carry admin source_type"
    );
  }

  #[tokio::test]
  #[serial]
  async fn test_get_domain_status_shows_event_history() {
    let (_db, pool) = setup_test_db().await;

    // Insert a few events for the domain without creating classifications.
    insert_event_at(&pool, "event-domain.com", "classifying", 30).await;
    insert_event_at(&pool, "event-domain.com", "error", 20).await;
    insert_event_at(&pool, "event-domain.com", "classifying", 10).await;
    insert_event_at(&pool, "event-domain.com", "classified", 5).await;

    let status = get_domain_status(&pool, "event-domain.com")
      .await
      .expect("get_domain_status failed");

    // domain was never inserted into `domains`, but events exist.
    assert!(!status.exists);
    assert_eq!(
      status.recent_events.len(),
      4,
      "should see all 4 events in history"
    );
    // Most recent event should be first.
    assert_eq!(status.recent_events[0].action, "classified");
    assert_eq!(status.recent_events[3].action, "classifying");
  }

  // ── apply_admin_classification tests ─────────────────────────────────────

  #[tokio::test]
  #[serial]
  async fn test_admin_classification_each_decision_has_own_source() {
    let (_db, pool) = setup_test_db().await;

    let admin_id: i32 =
      sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Two separate admin decisions should produce two distinct source rows.
    let source_a = apply_admin_classification(
      &pool,
      "site-a.com",
      "gaming",
      true,
      0.99,
      "Definitely gaming",
      admin_id,
      Some(30),
    )
    .await
    .unwrap();

    let source_b = apply_admin_classification(
      &pool,
      "site-b.com",
      "gaming",
      false,
      0.99,
      "Definitely not gaming",
      admin_id,
      Some(30),
    )
    .await
    .unwrap();

    assert_ne!(
      source_a, source_b,
      "each admin decision should have its own source row"
    );

    let count: i64 = sqlx::query_scalar(
      "SELECT COUNT(*) FROM classification_sources WHERE source_type = 'admin'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, 2, "two admin decisions → two source rows");
  }
}
