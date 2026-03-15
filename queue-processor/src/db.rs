use dns_smart_block_common::db_models::ClassificationEventInsert;
use sqlx::Postgres;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
  #[error("Database error: {0}")]
  SqlxError(#[from] sqlx::Error),

  #[error("JSON error: {0}")]
  JsonError(#[from] serde_json::Error),
}

/// Insert a domain_classification_event.  Accepts any Postgres executor so
/// callers can pass either a pool or an in-flight transaction.
pub async fn insert_event(
  executor: impl sqlx::Executor<'_, Database = Postgres>,
  domain: &str,
  action: &str,
  action_data: serde_json::Value,
  prompt_id: Option<i32>,
) -> Result<(), DbError> {
  let event = ClassificationEventInsert {
    domain: domain.to_string(),
    action: action.to_string(),
    action_data,
    prompt_id,
  };

  event.insert(executor).await?;

  Ok(())
}
