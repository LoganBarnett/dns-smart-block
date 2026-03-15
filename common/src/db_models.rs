use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

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

/// Input for inserting a domain classification event.
/// Contains only the fields we provide (not auto-generated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationEventInsert {
  pub domain: String,
  pub action: String,
  pub action_data: serde_json::Value,
  pub prompt_id: Option<i32>,
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

/// Full domain record as read from the database.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Domain {
  pub domain: String,
  pub last_updated: DateTime<Utc>,
}
