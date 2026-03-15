use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
  #[error("Database error: {0}")]
  SqlxError(#[from] sqlx::Error),

  #[error("JSON error: {0}")]
  JsonError(#[from] serde_json::Error),
}
