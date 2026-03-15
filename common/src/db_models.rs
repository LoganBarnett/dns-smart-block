use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgQueryResult, FromRow, Postgres, Transaction};

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
  /// Insert this event into the database
  pub async fn insert(
    &self,
    pool: &sqlx::PgPool,
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
    .execute(pool)
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
  /// Expire this domain.
  pub async fn expire(
    &self,
    tx: &mut Transaction<'_, Postgres>,
  ) -> Result<PgQueryResult, sqlx::Error> {
    let domain_classifications_expire = sqlx::query(
      r#"
      UPDATE domain_classifications
      SET valid_until = NOW()
      WHERE domain = $1 AND valid_on > NOW()
      "#,
    )
      .bind(&self.domain)
      .execute(&mut **tx);
    let domain_events_expire = sqlx::query(
      r#"

      "#,
    )
      .bind(&self.domain)
      .execute(&mut **tx);
    tokio::join!(domain_classifications_expire, domain_events_expire);
  }
}
/// Full domain record as read from the database.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Domain {
  pub domain: String,
  pub last_updated: DateTime<Utc>,
}
