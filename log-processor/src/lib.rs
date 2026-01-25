pub mod cli_args;
pub mod database_url;
pub mod db;
pub mod log_parser;
pub mod log_source;
pub mod queue;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProcessorError {
  #[error("IO error: {0}")]
  IoError(#[from] std::io::Error),

  #[error("HTTP request failed: {0}")]
  HttpError(#[from] reqwest::Error),

  #[error("JSON error: {0}")]
  JsonError(#[from] serde_json::Error),

  #[error("NATS error: {0}")]
  NatsError(String),

  #[error("Regex error: {0}")]
  RegexError(#[from] regex::Error),

  #[error("File watch error: {0}")]
  NotifyError(#[from] notify::Error),

  #[error("Invalid log source: {0}")]
  InvalidLogSource(String),

  #[error("Database error: {0}")]
  DatabaseError(#[from] db::DbError),

  #[error("SQL error: {0}")]
  SqlxError(#[from] sqlx::Error),

  #[error("Database URL error: {0}")]
  DatabaseUrlError(#[from] database_url::DatabaseUrlError),
}

pub type Result<T> = std::result::Result<T, ProcessorError>;
