use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Postgres, Row};
use std::collections::{HashMap, HashSet};

use super::error::DbError;
use super::models::{
  ActiveProvisionedPattern, ClassificationEventInsert, ClassificationInsert,
  ClassificationSource, DomainUpsert, PromptInsert,
};

/// Insert a domain_classification_event.  Accepts any Postgres executor so
/// callers can pass either a pool or an in-flight transaction.
pub async fn insert_event(
  executor: impl sqlx::Executor<'_, Database = Postgres>,
  domain: &str,
  action: &str,
  action_data: serde_json::Value,
  source_id: Option<i32>,
) -> Result<(), DbError> {
  ClassificationEventInsert {
    domain: domain.to_string(),
    action: action.to_string(),
    action_data,
    source_id,
  }
  .insert(executor)
  .await?;
  Ok(())
}

/// Store a classification result: ensures the prompt, ensures the
/// classification source, upserts the domain, and inserts a new projection
/// row — all within a single transaction.
///
/// Returns the source_id so callers can stamp audit events with the same
/// provenance (e.g. the classified event in the queue-processor).
///
/// This is the canonical way to persist a successful LLM classification.
pub async fn classification_store(
  pool: &PgPool,
  domain: &str,
  classification_type: &str,
  is_matching_site: bool,
  confidence: f64,
  reasoning: &str,
  model: &str,
  prompt_content: &str,
  prompt_hash: &str,
  ttl_days: i64,
) -> Result<i32, DbError> {
  let mut tx = pool.begin().await?;

  let prompt = PromptInsert {
    content: prompt_content.to_string(),
    hash: prompt_hash.to_string(),
  };
  let prompt_id = prompt.ensure(&mut tx).await?;

  let source_id =
    ClassificationSource::ensure_for_prompt(prompt_id, &mut tx).await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let valid_on = Utc::now();
  let valid_until = valid_on + Duration::days(ttl_days);

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site,
    confidence: confidence as f32,
    reasoning: Some(reasoning.to_string()),
    valid_on,
    valid_until,
    model: model.to_string(),
    source_id: Some(source_id),
  }
  .insert(&mut tx)
  .await?;

  tx.commit().await?;
  Ok(source_id)
}

/// Writes a synthetic "not matching" classification for a domain that matches
/// an exclude suffix, without invoking the LLM.  The domain upsert, source
/// ensure, projection insert, and audit event are all written atomically in a
/// single transaction so the record is always consistent.
pub async fn exclude_rule_classify(
  domain: &str,
  classification_type: &str,
  matched_suffix: &str,
  pool: &PgPool,
  ttl_days: i64,
) -> Result<(), DbError> {
  let reasoning = format!("Excluded by suffix: {}", matched_suffix);
  let now = Utc::now();

  let mut tx = pool.begin().await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let source_id = ClassificationSource::ensure_exclude_rule(
    "config_exclude_rule",
    matched_suffix,
    &mut tx,
  )
  .await?;

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site: false,
    confidence: 1.0,
    reasoning: Some(reasoning.clone()),
    valid_on: now,
    valid_until: now + Duration::days(ttl_days),
    model: "exclude-rule".to_string(),
    source_id: Some(source_id),
  }
  .insert(&mut tx)
  .await?;

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": classification_type,
      "is_matching_site": false,
      "confidence": 1.0,
      "reasoning": reasoning,
      "exclusion_suffix": matched_suffix,
      "model": "exclude-rule",
    }),
    Some(source_id),
  )
  .await?;

  tx.commit().await?;
  Ok(())
}

/// Writes a synthetic "not matching" classification for a domain that returned
/// NXDOMAIN during DNS resolution, without invoking the LLM.  The domain
/// upsert, source insert, projection, and audit event are written atomically.
pub async fn dns_nxdomain_classify(
  domain: &str,
  classification_type: &str,
  pool: &PgPool,
  ttl_days: i64,
) -> Result<(), DbError> {
  let now = Utc::now();
  let mut tx = pool.begin().await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let source_id = ClassificationSource::dns_nxdomain_insert(&mut tx).await?;

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site: false,
    confidence: 1.0,
    reasoning: Some("Domain does not exist (NXDOMAIN).".to_string()),
    valid_on: now,
    valid_until: now + Duration::days(ttl_days),
    model: "dns-nxdomain".to_string(),
    source_id: Some(source_id),
  }
  .insert(&mut tx)
  .await?;

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": classification_type,
      "is_matching_site": false,
      "confidence": 1.0,
      "reasoning": "Domain does not exist (NXDOMAIN).",
      "model": "dns-nxdomain",
    }),
    Some(source_id),
  )
  .await?;

  tx.commit().await?;
  Ok(())
}

/// Write a manual admin classification for a domain.  Creates an `admin`
/// source row tied to the given user, inserts a projection, and appends a
/// `classified` event — all atomically in one transaction.
///
/// Returns the `source_id` of the newly created admin source.
/// Sentinel `valid_until` used when a classification should never expire.
/// Approximately year 3026 — far enough to be "forever" without needing a
/// nullable column or schema change.
fn never_expires_at(now: DateTime<Utc>) -> DateTime<Utc> {
  now + Duration::days(365_000)
}

pub async fn apply_admin_classification(
  pool: &PgPool,
  domain: &str,
  classification_type: &str,
  is_matching_site: bool,
  confidence: f64,
  reasoning: &str,
  user_id: i32,
  ttl_days: Option<i64>,
) -> Result<i32, DbError> {
  let mut tx = pool.begin().await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  let source_id = ClassificationSource::admin_insert(user_id, &mut tx).await?;

  let now = Utc::now();
  let valid_until = match ttl_days {
    Some(d) => now + Duration::days(d),
    None => never_expires_at(now),
  };

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site,
    confidence: confidence as f32,
    reasoning: Some(reasoning.to_string()),
    valid_on: now,
    valid_until,
    model: "admin".to_string(),
    source_id: Some(source_id),
  }
  .insert(&mut tx)
  .await?;

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": classification_type,
      "is_matching_site": is_matching_site,
      "confidence": confidence,
      "reasoning": reasoning,
      "model": "admin",
    }),
    Some(source_id),
  )
  .await?;

  tx.commit().await?;
  Ok(source_id)
}

/// A single desired provisioned classification entry, as declared in NixOS
/// configuration and submitted via `dns-smart-block-cli domain reconcile`.
///
/// Exactly one of `domain` or `pattern` must be set.  Use `domain` for an
/// exact-match classification of a single registrable domain; use `pattern`
/// for a POSIX/Rust-regex that should cover a family of domains (e.g.
/// `^(.*\.)?dndbeyond\.com$`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionedEntry {
  #[serde(default)]
  pub domain: Option<String>,
  #[serde(default)]
  pub pattern: Option<String>,
  pub classification_type: String,
  pub is_matching_site: bool,
  pub confidence: f64,
  pub reasoning: Option<String>,
}

/// Summary of what a reconcile run changed.
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileResult {
  /// Entries that were inserted or updated.
  pub upserted: usize,
  /// Entries whose active row already matched — no write needed.
  pub skipped: usize,
  /// Previously-provisioned classifications removed from the declared set.
  pub expired: usize,
}

/// Reconcile the `provisioned` classification source against a desired set.
///
/// - Entries in `desired` are upserted (insert new or expire-and-reinsert if
///   values changed).  Unchanged active rows are skipped.
/// - Active `provisioned` rows whose `(domain, classification_type)` is not
///   in `desired` are expired and an `expired` event is written.
/// - `admin`-sourced rows are never touched.
pub async fn reconcile_provisioned_classifications(
  pool: &PgPool,
  desired: &[ProvisionedEntry],
) -> Result<ReconcileResult, DbError> {
  let mut tx = pool.begin().await?;
  let now = Utc::now();

  // Ensure the singleton provisioned source row exists.
  let source_id = ClassificationSource::ensure_provisioned(&mut tx).await?;

  // Collect all currently-active provisioned rows.
  let active_rows = sqlx::query(
    r#"
    SELECT domain, classification_type, is_matching_site, confidence, reasoning
    FROM domain_classifications
    WHERE source_id = $1 AND valid_until > $2
    "#,
  )
  .bind(source_id)
  .bind(now)
  .fetch_all(&mut *tx)
  .await?;

  // Build a lookup: (domain, classification_type) → (is_matching_site, confidence, reasoning)
  let mut active_map: HashMap<(String, String), (bool, f32, Option<String>)> =
    HashMap::new();
  for row in &active_rows {
    let domain: String = row.try_get("domain")?;
    let ct: String = row.try_get("classification_type")?;
    let is_matching: bool = row.try_get("is_matching_site")?;
    let confidence: f32 = row.try_get("confidence")?;
    let reasoning: Option<String> = row.try_get("reasoning")?;
    active_map.insert((domain, ct), (is_matching, confidence, reasoning));
  }

  let desired_keys: HashSet<(String, String)> = desired
    .iter()
    .filter_map(|e| {
      e.domain
        .as_ref()
        .map(|d| (d.clone(), e.classification_type.clone()))
    })
    .collect();

  let mut upserted = 0usize;
  let mut skipped = 0usize;

  for entry in desired {
    let domain = match &entry.domain {
      Some(d) => d.clone(),
      None => continue, // pattern entries are handled by reconcile_provisioned_patterns
    };
    let key = (domain.clone(), entry.classification_type.clone());

    // Skip if the active row already matches desired values.
    if let Some(&(existing_matching, existing_conf, ref existing_reasoning)) =
      active_map.get(&key)
    {
      let same_values = existing_matching == entry.is_matching_site
        && (existing_conf - entry.confidence as f32).abs() < 0.0001
        && existing_reasoning.as_deref().unwrap_or("")
          == entry.reasoning.as_deref().unwrap_or("");

      if same_values {
        skipped += 1;
        continue;
      }

      // Values changed — expire the old row before inserting a fresh one.
      sqlx::query(
        r#"
        UPDATE domain_classifications SET valid_until = $1
        WHERE domain = $2 AND classification_type = $3
          AND source_id = $4 AND valid_until > $1
        "#,
      )
      .bind(now)
      .bind(&domain)
      .bind(&entry.classification_type)
      .bind(source_id)
      .execute(&mut *tx)
      .await?;
    }

    // Expire any active LLM-sourced classifications for this (domain, type)
    // so the provisioned row wins over stale LLM results.
    sqlx::query(
      r#"
      UPDATE domain_classifications SET valid_until = $1
      WHERE domain = $2 AND classification_type = $3
        AND valid_until > $1
        AND source_id IN (
          SELECT id FROM classification_sources
          WHERE source_type IN ('llm_prompt', 'config_exclude_rule', 'manual_exclude_rule')
        )
      "#,
    )
    .bind(now)
    .bind(&domain)
    .bind(&entry.classification_type)
    .execute(&mut *tx)
    .await?;

    DomainUpsert {
      domain: domain.clone(),
    }
    .upsert(&mut tx)
    .await?;

    ClassificationInsert {
      domain: domain.clone(),
      classification_type: entry.classification_type.clone(),
      is_matching_site: entry.is_matching_site,
      confidence: entry.confidence as f32,
      reasoning: entry.reasoning.clone(),
      valid_on: now,
      valid_until: never_expires_at(now),
      model: "provisioned".to_string(),
      source_id: Some(source_id),
    }
    .insert(&mut tx)
    .await?;

    insert_event(
      &mut *tx,
      &domain,
      "classified",
      json!({
        "classification_type": entry.classification_type,
        "is_matching_site": entry.is_matching_site,
        "confidence": entry.confidence,
        "reasoning": entry.reasoning.as_deref().unwrap_or(""),
        "model": "provisioned",
      }),
      Some(source_id),
    )
    .await?;

    upserted += 1;
  }

  // Expire any active provisioned rows that are no longer in the desired set.
  let mut expired = 0usize;
  for (domain, ct) in active_map.keys() {
    if !desired_keys.contains(&(domain.clone(), ct.clone())) {
      sqlx::query(
        r#"
        UPDATE domain_classifications SET valid_until = $1
        WHERE domain = $2 AND classification_type = $3
          AND source_id = $4 AND valid_until > $1
        "#,
      )
      .bind(now)
      .bind(domain)
      .bind(ct)
      .bind(source_id)
      .execute(&mut *tx)
      .await?;

      insert_event(
        &mut *tx,
        domain,
        "expired",
        json!({
          "classification_type": ct,
          "reason": "removed from provisioned configuration",
        }),
        Some(source_id),
      )
      .await?;

      expired += 1;
    }
  }

  tx.commit().await?;
  Ok(ReconcileResult {
    upserted,
    skipped,
    expired,
  })
}

/// Apply a provisioned-pattern classification for a single domain.
///
/// This is used both during `reconcile_provisioned_patterns` (retroactive
/// application to existing domains) and by the queue-processor (to skip the
/// LLM when a domain matches an active pattern rule at processing time).
///
/// - Upserts the domain.
/// - Expires any active LLM-sourced or exclude-rule classifications for
///   `(domain, classification_type)` so the pattern row wins.
/// - Inserts a fresh `domain_classifications` row that never expires.
/// - Inserts a `classified` event.
pub async fn apply_pattern_classification(
  pool: &PgPool,
  domain: &str,
  classification_type: &str,
  pattern_rule: &ActiveProvisionedPattern,
) -> Result<(), DbError> {
  let now = Utc::now();
  let mut tx = pool.begin().await?;

  DomainUpsert {
    domain: domain.to_string(),
  }
  .upsert(&mut tx)
  .await?;

  // Expire active LLM / exclude-rule classifications so the pattern wins.
  sqlx::query(
    r#"
    UPDATE domain_classifications SET valid_until = $1
    WHERE domain = $2 AND classification_type = $3
      AND valid_until > $1
      AND source_id IN (
        SELECT id FROM classification_sources
        WHERE source_type IN (
          'llm_prompt', 'config_exclude_rule', 'manual_exclude_rule'
        )
      )
    "#,
  )
  .bind(now)
  .bind(domain)
  .bind(classification_type)
  .execute(&mut *tx)
  .await?;

  ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site: pattern_rule.is_matching_site,
    confidence: pattern_rule.confidence,
    reasoning: pattern_rule.reasoning.clone(),
    valid_on: now,
    valid_until: never_expires_at(now),
    model: "provisioned-pattern".to_string(),
    source_id: pattern_rule.source_id,
  }
  .insert(&mut tx)
  .await?;

  insert_event(
    &mut *tx,
    domain,
    "classified",
    json!({
      "classification_type": classification_type,
      "is_matching_site": pattern_rule.is_matching_site,
      "confidence": pattern_rule.confidence,
      "reasoning": pattern_rule.reasoning.as_deref().unwrap_or(""),
      "model": "provisioned-pattern",
      "matched_pattern": pattern_rule.pattern,
    }),
    pattern_rule.source_id,
  )
  .await?;

  tx.commit().await?;
  Ok(())
}

/// Reconcile the `provisioned_pattern_rules` table against a desired set of
/// pattern entries.
///
/// - Desired patterns that are new or changed are upserted (old rule expired,
///   new rule inserted).
/// - For each new/changed pattern, all existing domains in the database that
///   match the regex are immediately reclassified.
/// - Rules that are no longer in the desired set are expired.
/// - Admin-sourced pattern rules are never touched.
pub async fn reconcile_provisioned_patterns(
  pool: &PgPool,
  desired: &[ProvisionedEntry],
) -> Result<ReconcileResult, DbError> {
  let now = Utc::now();

  // Fetch currently-active provisioned_pattern rules.
  let active_rules: Vec<(
    i32,
    String,
    String,
    bool,
    f32,
    Option<String>,
    Option<i32>,
  )> = sqlx::query_as(
    r#"
      SELECT ppr.id, ppr.pattern, ppr.classification_type, ppr.is_matching_site,
             ppr.confidence, ppr.reasoning, ppr.source_id
      FROM provisioned_pattern_rules ppr
      JOIN classification_sources cs ON ppr.source_id = cs.id
      WHERE cs.source_type = 'provisioned_pattern'
        AND ppr.valid_on <= $1 AND ppr.valid_until > $1
      "#,
  )
  .bind(now)
  .fetch_all(pool)
  .await?;

  let mut active_map: std::collections::HashMap<
    (String, String),
    (i32, bool, f32, Option<String>, Option<i32>),
  > = std::collections::HashMap::new();
  for (id, pattern, ct, is_matching, confidence, reasoning, source_id) in
    &active_rules
  {
    active_map.insert(
      (pattern.clone(), ct.clone()),
      (
        *id,
        *is_matching,
        *confidence,
        reasoning.clone(),
        *source_id,
      ),
    );
  }

  let desired_keys: std::collections::HashSet<(String, String)> = desired
    .iter()
    .filter_map(|e| {
      e.pattern
        .as_ref()
        .map(|p| (p.clone(), e.classification_type.clone()))
    })
    .collect();

  let mut upserted = 0usize;
  let mut skipped = 0usize;
  let mut expired = 0usize;

  for entry in desired {
    let pattern = match &entry.pattern {
      Some(p) => p,
      None => continue, // domain entries handled by reconcile_provisioned_classifications
    };

    let key = (pattern.clone(), entry.classification_type.clone());

    if let Some((
      _,
      existing_matching,
      existing_conf,
      ref existing_reasoning,
      _,
    )) = active_map.get(&key)
    {
      let same = *existing_matching == entry.is_matching_site
        && (*existing_conf - entry.confidence as f32).abs() < 0.0001
        && existing_reasoning.as_deref().unwrap_or("")
          == entry.reasoning.as_deref().unwrap_or("");

      if same {
        skipped += 1;
        continue;
      }

      // Values changed — expire the old rule row.
      let old_id = active_map[&key].0;
      sqlx::query(
        "UPDATE provisioned_pattern_rules SET valid_until = $1 WHERE id = $2",
      )
      .bind(now)
      .bind(old_id)
      .execute(pool)
      .await?;
    }

    // Ensure the classification_sources row for this pattern exists.
    let mut tx = pool.begin().await?;
    let source_id = ClassificationSource::ensure_provisioned_pattern(
      pattern,
      &entry.classification_type,
      &mut tx,
    )
    .await?;

    // Insert the new pattern rule.
    sqlx::query(
      r#"
      INSERT INTO provisioned_pattern_rules
        (pattern, classification_type, is_matching_site, confidence, reasoning,
         source_id, valid_on, valid_until, created_at)
      VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
      "#,
    )
    .bind(pattern)
    .bind(&entry.classification_type)
    .bind(entry.is_matching_site)
    .bind(entry.confidence as f32)
    .bind(entry.reasoning.as_deref())
    .bind(source_id)
    .bind(now)
    .bind(never_expires_at(now))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // Build the ActiveProvisionedPattern to pass to apply_pattern_classification.
    let rule = ActiveProvisionedPattern {
      id: 0, // not needed for apply
      pattern: pattern.clone(),
      classification_type: entry.classification_type.clone(),
      is_matching_site: entry.is_matching_site,
      confidence: entry.confidence as f32,
      reasoning: entry.reasoning.clone(),
      source_id: Some(source_id),
    };

    // Retroactively apply to all existing matching domains.
    let re = regex::Regex::new(pattern)
      .map_err(|e| DbError::RegexError(e.to_string()))?;

    let all_domains: Vec<String> =
      sqlx::query_scalar("SELECT domain FROM domains")
        .fetch_all(pool)
        .await?;

    for domain in all_domains {
      if re.is_match(&domain) {
        apply_pattern_classification(
          pool,
          &domain,
          &entry.classification_type,
          &rule,
        )
        .await?;
      }
    }

    upserted += 1;
  }

  // Expire any active provisioned pattern rules no longer in the desired set.
  for ((pattern, ct), (rule_id, _, _, _, _)) in &active_map {
    if !desired_keys.contains(&(pattern.clone(), ct.clone())) {
      sqlx::query(
        "UPDATE provisioned_pattern_rules SET valid_until = $1 WHERE id = $2",
      )
      .bind(now)
      .bind(rule_id)
      .execute(pool)
      .await?;
      expired += 1;
    }
  }

  Ok(ReconcileResult {
    upserted,
    skipped,
    expired,
  })
}

/// Top-level reconcile: routes domain-based entries to
/// `reconcile_provisioned_classifications` and pattern-based entries to
/// `reconcile_provisioned_patterns`, then sums the results.
pub async fn reconcile_all_provisioned(
  pool: &PgPool,
  entries: &[ProvisionedEntry],
) -> Result<ReconcileResult, DbError> {
  let domain_result =
    reconcile_provisioned_classifications(pool, entries).await?;
  let pattern_result = reconcile_provisioned_patterns(pool, entries).await?;

  Ok(ReconcileResult {
    upserted: domain_result.upserted + pattern_result.upserted,
    skipped: domain_result.skipped + pattern_result.skipped,
    expired: domain_result.expired + pattern_result.expired,
  })
}

/// Write a pattern-based admin classification.  Similar to
/// `reconcile_provisioned_patterns` but uses an `admin` source rather than
/// `provisioned_pattern`, so reconcile never touches it.
///
/// Inserts a new `provisioned_pattern_rules` row (sourced from the given
/// admin user) and immediately applies the classification to all existing
/// domains in the database that match the regex.
///
/// Returns the source_id of the newly created admin source.
pub async fn apply_admin_pattern_classification(
  pool: &PgPool,
  pattern: &str,
  classification_type: &str,
  is_matching_site: bool,
  confidence: f64,
  reasoning: &str,
  user_id: i32,
) -> Result<i32, DbError> {
  let now = Utc::now();

  let mut tx = pool.begin().await?;
  let source_id = ClassificationSource::admin_insert(user_id, &mut tx).await?;

  sqlx::query(
    r#"
    INSERT INTO provisioned_pattern_rules
      (pattern, classification_type, is_matching_site, confidence, reasoning,
       source_id, valid_on, valid_until, created_at)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
    "#,
  )
  .bind(pattern)
  .bind(classification_type)
  .bind(is_matching_site)
  .bind(confidence as f32)
  .bind(reasoning)
  .bind(source_id)
  .bind(now)
  .bind(never_expires_at(now))
  .execute(&mut *tx)
  .await?;

  tx.commit().await?;

  let rule = ActiveProvisionedPattern {
    id: 0,
    pattern: pattern.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site,
    confidence: confidence as f32,
    reasoning: Some(reasoning.to_string()),
    source_id: Some(source_id),
  };

  let re = regex::Regex::new(pattern)
    .map_err(|e| DbError::RegexError(e.to_string()))?;

  let all_domains: Vec<String> =
    sqlx::query_scalar("SELECT domain FROM domains")
      .fetch_all(pool)
      .await?;

  for domain in all_domains {
    if re.is_match(&domain) {
      apply_pattern_classification(pool, &domain, classification_type, &rule)
        .await?;
    }
  }

  Ok(source_id)
}

/// Rebuild the `domain_classifications` projection from the event log.
/// Processes all "classified" events and recreates projection rows.
/// Returns the count of projections written.
pub async fn rebuild_projections_from_events(
  pool: &PgPool,
  ttl_days: i64,
) -> Result<i64, DbError> {
  let rows = sqlx::query(
    r#"
    WITH latest_classified_events AS (
        SELECT DISTINCT ON (domain, (action_data->>'classification_type'))
            domain,
            action_data->>'classification_type' as classification_type,
            (action_data->>'is_matching_site')::boolean as is_matching_site,
            (action_data->>'confidence')::real as confidence,
            action_data->>'reasoning' as reasoning,
            COALESCE(action_data->>'model', 'unknown') as model,
            source_id,
            created_at
        FROM domain_classification_events
        WHERE action = 'classified'
        ORDER BY domain, (action_data->>'classification_type'), created_at DESC
    )
    SELECT
        domain,
        classification_type,
        is_matching_site,
        confidence,
        reasoning,
        model,
        source_id,
        created_at
    FROM latest_classified_events
    WHERE classification_type IS NOT NULL
      AND is_matching_site IS NOT NULL
      AND confidence IS NOT NULL
    "#,
  )
  .fetch_all(pool)
  .await?;

  let mut tx = pool.begin().await?;
  let mut count = 0i64;

  sqlx::query("DELETE FROM domain_classifications")
    .execute(&mut *tx)
    .await?;

  for row in rows {
    let domain: String = row.try_get("domain")?;
    let classification_type: String = row.try_get("classification_type")?;
    let is_matching_site: bool = row.try_get("is_matching_site")?;
    let confidence: f32 = row.try_get("confidence")?;
    let reasoning: Option<String> = row.try_get("reasoning")?;
    let model: String = row.try_get("model")?;
    let source_id: Option<i32> = row.try_get("source_id")?;
    let event_created_at: DateTime<Utc> = row.try_get("created_at")?;

    let valid_on = event_created_at;
    let valid_until = valid_on + Duration::days(ttl_days);

    DomainUpsert {
      domain: domain.clone(),
    }
    .upsert(&mut tx)
    .await?;

    ClassificationInsert {
      domain,
      classification_type,
      is_matching_site,
      confidence,
      reasoning,
      valid_on,
      valid_until,
      model,
      source_id,
    }
    .insert_with_created_at(&mut tx, event_created_at)
    .await?;

    count += 1;
  }

  tx.commit().await?;
  Ok(count)
}
