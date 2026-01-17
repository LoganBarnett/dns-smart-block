pub mod cli_args;
pub mod dnsdist;
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
}

pub type Result<T> = std::result::Result<T, ProcessorError>;
