use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{
  FromRow, PgPool, Postgres, Row, Transaction, postgres::PgQueryResult,
};

/// Input for inserting a domain classification projection.
/// Contains only the fields we provide (not auto-generated like created_at).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationInsert {
  pub domain: String,
  pub classification_type: String,
  pub is_matching_site: bool,
  pub confidence: f32,
  pub reasoning: Option<String>,
  pub valid_on: DateTime<Utc>,
  pub valid_until: DateTime<Utc>,
  pub model: String,
  pub prompt_id: Option<i32>,
}

impl ClassificationInsert {
  /// Insert this classification into the database
  pub async fn insert(
    &self,
    tx: &mut Transaction<'_, Postgres>,
  ) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
      r#"
      INSERT INTO domain_classifications (
        domain, classification_type, is_matching_site, confidence, reasoning,
        valid_on, valid_until, model, prompt_id, created_at
      )
      VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
      "#,
    )
    .bind(&self.domain)
    .bind(&self.classification_type)
    .bind(self.is_matching_site)
    .bind(self.confidence)
    .bind(&self.reasoning)
    .bind(self.valid_on)
    .bind(self.valid_until)
    .bind(&self.model)
    .bind(self.prompt_id)
    .execute(&mut **tx)
    .await
  }

  /// Insert with an explicit created_at timestamp (for reprojection)
  pub async fn insert_with_created_at(
    &self,
    tx: &mut Transaction<'_, Postgres>,
    created_at: DateTime<Utc>,
  ) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
      r#"
            INSERT INTO domain_classifications (
                domain, classification_type, is_matching_site, confidence, reasoning, valid_on, valid_until, model, prompt_id, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
    )
    .bind(&self.domain)
    .bind(&self.classification_type)
    .bind(self.is_matching_site)
    .bind(self.confidence)
    .bind(&self.reasoning)
    .bind(self.valid_on)
    .bind(self.valid_until)
    .bind(&self.model)
    .bind(self.prompt_id)
    .bind(created_at)
    .execute(&mut **tx)
    .await
  }
}

/// Full domain classification record as read from the database.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Classification {
  pub id: i32,
  pub domain: String,
  pub classification_type: String,
  pub is_matching_site: bool,
  pub confidence: f32,
  pub reasoning: Option<String>,
  pub valid_on: DateTime<Utc>,
  pub valid_until: DateTime<Utc>,
  pub model: String,
  pub prompt_id: Option<i32>,
  pub created_at: DateTime<Utc>,
}

impl Classification {
  /// Query blocked domains for a given classification type at a specific time
  pub async fn find_blocked_domains(
    pool: &sqlx::PgPool,
    classification_type: &str,
    at_time: Option<DateTime<Utc>>,
  ) -> Result<Vec<String>, sqlx::Error> {
    let at_time = at_time.unwrap_or_else(Utc::now);

    let domains: Vec<(String,)> = sqlx::query_as(
      r#"
      SELECT DISTINCT domain
      FROM domain_classifications
      WHERE classification_type = $1
        AND is_matching_site = true
        AND valid_on <= $2
        AND valid_until > $2
      "#,
    )
    .bind(classification_type)
    .bind(at_time)
    .fetch_all(pool)
    .await?;

    Ok(domains.into_iter().map(|(d,)| d).collect())
  }
}

/// Input for inserting a domain classification event.
/// Contains only the fields we provide (not auto-generated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationEventInsert {
  pub domain: String,
  pub action: String,
  pub action_data: serde_json::Value,
  pub prompt_id: Option<i32>,
}

impl ClassificationEventInsert {
  /// Insert this event into the database.  Accepts any Postgres executor so
  /// callers can pass either a pool or an in-flight transaction.
  pub async fn insert(
    &self,
    executor: impl sqlx::Executor<'_, Database = Postgres>,
  ) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
      r#"
      INSERT INTO domain_classification_events (domain, action, action_data, prompt_id, created_at)
      VALUES ($1, $2::classification_action, $3, $4, NOW())
      "#,
    )
    .bind(&self.domain)
    .bind(&self.action)
    .bind(&self.action_data)
    .bind(self.prompt_id)
    .execute(executor)
    .await
  }
}

/// Full domain classification event record as read from the database.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ClassificationEvent {
  pub id: i32,
  pub domain: String,
  pub action: String,
  pub action_data: serde_json::Value,
  pub prompt_id: Option<i32>,
  pub created_at: DateTime<Utc>,
}

impl ClassificationEvent {
  /// Return the latest event for a domain, if any.
  pub async fn domain_latest(
    pool: &PgPool,
    domain: &str,
  ) -> Result<Option<(String, serde_json::Value)>, sqlx::Error> {
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
    .fetch_optional(pool)
    .await?;

    match result {
      Some(row) => {
        let action: String = row.try_get("action")?;
        let action_data: serde_json::Value = row.try_get("action_data")?;
        Ok(Some((action, action_data)))
      }
      None => Ok(None),
    }
  }

  /// Count consecutive `error` events for a domain, from the most recent
  /// event backwards (up to `limit` events).
  pub async fn consecutive_error_count(
    pool: &PgPool,
    domain: &str,
    limit: i64,
  ) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
      r#"
      WITH recent_events AS (
          SELECT
              action::text,
              ROW_NUMBER() OVER (ORDER BY created_at DESC) as rn
          FROM domain_classification_events
          WHERE domain = $1
          ORDER BY created_at DESC
          LIMIT $2
      )
      SELECT COUNT(*) as error_count
      FROM recent_events
      WHERE action = 'error'
      AND NOT EXISTS (
          SELECT 1
          FROM recent_events re2
          WHERE re2.action != 'error'
          AND re2.rn <= recent_events.rn
      )
      "#,
    )
    .bind(domain)
    .bind(limit)
    .fetch_one(pool)
    .await?;

    let count: i64 = result.try_get("error_count")?;
    Ok(count)
  }
}

/// Classification state for a specific classifier type.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassifierState {
  /// Classification is current (within TTL, not in error state)
  Current,
  /// Classification exists but is expired (past valid_until)
  Expired,
  /// Most recent event for this type is an error
  Error,
  /// No classification exists for this type
  Missing,
}

impl ClassifierState {
  /// Return the state of each classification type for a domain in a single
  /// query.
  pub async fn domain_states(
    pool: &PgPool,
    domain: &str,
    classification_types: &[String],
  ) -> Result<Vec<(String, ClassifierState)>, sqlx::Error> {
    let result = sqlx::query(
      r#"
      WITH classification_types AS (
          SELECT unnest($2::text[]) AS classification_type
      ),
      latest_classifications AS (
          SELECT DISTINCT ON (dc.classification_type)
              dc.classification_type,
              dc.valid_until,
              dc.valid_until > NOW() AS is_valid
          FROM domain_classifications dc
          WHERE dc.domain = $1
          ORDER BY dc.classification_type, dc.created_at DESC
      ),
      latest_events AS (
          SELECT DISTINCT ON (action_data->>'classification_type')
              action_data->>'classification_type' AS classification_type,
              action::text AS action
          FROM domain_classification_events
          WHERE domain = $1
            AND action_data->>'classification_type' IS NOT NULL
          ORDER BY action_data->>'classification_type', created_at DESC
      )
      SELECT
          ct.classification_type,
          lc.is_valid,
          le.action
      FROM classification_types ct
      LEFT JOIN latest_classifications lc
          ON ct.classification_type = lc.classification_type
      LEFT JOIN latest_events le
          ON ct.classification_type = le.classification_type
      "#,
    )
    .bind(domain)
    .bind(classification_types)
    .fetch_all(pool)
    .await?;

    let mut states = Vec::new();

    for row in result {
      let classification_type: String = row.try_get("classification_type")?;
      let is_valid: Option<bool> = row.try_get("is_valid").ok();
      let latest_action: Option<String> = row.try_get("action").ok();

      let state = match (is_valid, latest_action.as_deref()) {
        (Some(true), _) => ClassifierState::Current,
        (Some(false), _) => ClassifierState::Expired,
        (None, Some("error")) => ClassifierState::Error,
        (None, _) => ClassifierState::Missing,
      };

      states.push((classification_type, state));
    }

    Ok(states)
  }
}

/// Input for inserting or ensuring a prompt exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptInsert {
  pub content: String,
  pub hash: String,
}

impl PromptInsert {
  /// Ensure this prompt exists in the database, returning its ID.
  /// Uses INSERT ... ON CONFLICT DO NOTHING to handle duplicates.
  pub async fn ensure(
    &self,
    tx: &mut Transaction<'_, Postgres>,
  ) -> Result<i32, sqlx::Error> {
    // Try to insert, ignore conflicts
    sqlx::query(
      r#"
            INSERT INTO prompts (content, hash, created_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (hash) DO NOTHING
            "#,
    )
    .bind(&self.content)
    .bind(&self.hash)
    .execute(&mut **tx)
    .await?;

    // Get the ID
    let result: (i32,) = sqlx::query_as(
      r#"
            SELECT id FROM prompts WHERE hash = $1
            "#,
    )
    .bind(&self.hash)
    .fetch_one(&mut **tx)
    .await?;

    Ok(result.0)
  }
}

/// Full prompt record as read from the database.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Prompt {
  pub id: i32,
  pub content: String,
  pub hash: String,
  pub created_at: DateTime<Utc>,
}

/// Input for upserting a domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainUpsert {
  pub domain: String,
}

impl DomainUpsert {
  /// Upsert this domain into the database
  pub async fn upsert(
    &self,
    tx: &mut Transaction<'_, Postgres>,
  ) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
      r#"
            INSERT INTO domains (domain, last_updated)
            VALUES ($1, NOW())
            ON CONFLICT (domain) DO UPDATE SET last_updated = NOW()
            "#,
    )
    .bind(&self.domain)
    .execute(&mut **tx)
    .await
  }
}

/// Input for expiring a domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainExpire {
  pub domain: String,
}

impl DomainExpire {
  /// Expire all currently valid classifications for this domain and record
  /// an `expired` event. Both the projection update and event insertion
  /// share the same timestamp so the event log stays consistent.
  pub async fn expire(
    &self,
    tx: &mut Transaction<'_, Postgres>,
  ) -> Result<PgQueryResult, sqlx::Error> {
    let now = Utc::now();

    let result = sqlx::query(
      r#"
      UPDATE domain_classifications
      SET valid_until = $1
      WHERE domain = $2
        AND valid_on <= $1
        AND valid_until > $1
      "#,
    )
    .bind(now)
    .bind(&self.domain)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
      r#"
      INSERT INTO domain_classification_events (domain, action, action_data, created_at)
      VALUES ($1, 'expired'::classification_action, '{}'::jsonb, $2)
      "#,
    )
    .bind(&self.domain)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(result)
  }
}

/// Marks a domain for re-classification by inserting a `queued` event.
///
/// Inserting this event signals the log-processor not to double-queue the
/// domain while it is already in-flight via NATS, and gives the event log
/// an audit trail for the admin requeue action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainRequeue {
  pub domain: String,
}

impl DomainRequeue {
  /// Insert a `queued` event for the domain.
  pub async fn requeue(
    &self,
    tx: &mut Transaction<'_, Postgres>,
  ) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    sqlx::query(
      r#"
      INSERT INTO domain_classification_events (domain, action, action_data, created_at)
      VALUES ($1, 'queued'::classification_action, '{}'::jsonb, $2)
      "#,
    )
    .bind(&self.domain)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
  }
}

/// A (domain, classification_type) pair whose most recent type-specific event
/// is an `error`. Used by the admin UI to surface and requeue failed
/// classifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErroredClassification {
  pub domain: String,
  pub classification_type: String,
  pub error_message: Option<String>,
  pub errored_at: DateTime<Utc>,
}

impl ErroredClassification {
  /// Return all errored (domain, classification_type) pairs, optionally
  /// filtered to a single classification type.
  ///
  /// A pair is "errored" when the most recent event that carries a
  /// `classification_type` field in its `action_data` is an `error` event.
  pub async fn find(
    pool: &sqlx::PgPool,
    classification_type: Option<&str>,
  ) -> Result<Vec<Self>, sqlx::Error> {
    let rows = if let Some(ct) = classification_type {
      sqlx::query(
        r#"
        WITH latest_type_events AS (
            SELECT DISTINCT ON (domain, action_data->>'classification_type')
                domain,
                action_data->>'classification_type' AS classification_type,
                action::text AS action,
                action_data->>'error' AS error_message,
                created_at
            FROM domain_classification_events
            WHERE action_data->>'classification_type' IS NOT NULL
              AND action_data->>'classification_type' = $1
            ORDER BY domain, action_data->>'classification_type', created_at DESC
        )
        SELECT domain, classification_type, error_message, created_at
        FROM latest_type_events
        WHERE action = 'error'
        ORDER BY created_at DESC
        "#,
      )
      .bind(ct)
      .fetch_all(pool)
      .await?
    } else {
      sqlx::query(
        r#"
        WITH latest_type_events AS (
            SELECT DISTINCT ON (domain, action_data->>'classification_type')
                domain,
                action_data->>'classification_type' AS classification_type,
                action::text AS action,
                action_data->>'error' AS error_message,
                created_at
            FROM domain_classification_events
            WHERE action_data->>'classification_type' IS NOT NULL
            ORDER BY domain, action_data->>'classification_type', created_at DESC
        )
        SELECT domain, classification_type, error_message, created_at
        FROM latest_type_events
        WHERE action = 'error'
        ORDER BY created_at DESC
        "#,
      )
      .fetch_all(pool)
      .await?
    };

    rows
      .into_iter()
      .map(|row| {
        use sqlx::Row;
        Ok(Self {
          domain: row.try_get("domain")?,
          classification_type: row.try_get("classification_type")?,
          error_message: row.try_get("error_message")?,
          errored_at: row.try_get("created_at")?,
        })
      })
      .collect()
  }
}
/// Full domain record as read from the database.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Domain {
  pub domain: String,
  pub last_updated: DateTime<Utc>,
}

/// Store a classification result: upserts the prompt, upserts the domain,
/// inserts a new projection row, all within a single transaction.
///
/// This is the canonical way to persist a successful classification result.
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
) -> Result<(), sqlx::Error> {
  let mut tx = pool.begin().await?;

  let prompt = PromptInsert {
    content: prompt_content.to_string(),
    hash: prompt_hash.to_string(),
  };
  let prompt_id = prompt.ensure(&mut tx).await?;

  let domain_upsert = DomainUpsert {
    domain: domain.to_string(),
  };
  domain_upsert.upsert(&mut tx).await?;

  let valid_on = Utc::now();
  let valid_until = valid_on + Duration::days(ttl_days);

  let classification = ClassificationInsert {
    domain: domain.to_string(),
    classification_type: classification_type.to_string(),
    is_matching_site,
    confidence: confidence as f32,
    reasoning: Some(reasoning.to_string()),
    valid_on,
    valid_until,
    model: model.to_string(),
    prompt_id: Some(prompt_id),
  };
  classification.insert(&mut tx).await?;

  tx.commit().await?;
  Ok(())
}
