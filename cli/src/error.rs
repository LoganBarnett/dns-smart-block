use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
  #[error("HTTP request failed: {0}")]
  Http(#[from] reqwest::Error),

  #[error("API error (HTTP {status}): {body}")]
  Api { status: u16, body: String },

  #[error(
    "--ensure is required.  Use --ensure to explicitly request create-or-update behavior."
  )]
  EnsureRequired,
}
