use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkerError {
  #[error("HTTP fetch failed: {0}")]
  HttpError(#[from] reqwest::Error),

  #[error("IO error: {0}")]
  IoError(#[from] std::io::Error),

  #[error("JSON error: {0}")]
  JsonError(#[from] serde_json::Error),
}
