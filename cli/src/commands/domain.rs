use crate::error::CliError;
use clap::{Args, Subcommand};
use serde::Serialize;
use tracing::info;

#[derive(Args, Debug)]
pub struct DomainArgs {
  #[command(subcommand)]
  pub command: DomainCommands,
}

#[derive(Subcommand, Debug)]
pub enum DomainCommands {
  /// Apply an admin classification for a domain
  Classify(ClassifyArgs),
}

#[derive(Args, Debug)]
pub struct ClassifyArgs {
  /// Domain to classify (e.g. "example.com")
  #[arg(long)]
  pub domain: String,

  /// Classification type (e.g. "gaming", "news")
  #[arg(long)]
  pub classification_type: String,

  /// Whether the domain matches this classification category
  #[arg(long)]
  pub is_matching_site: bool,

  /// Confidence score (0.0–1.0)
  #[arg(long, default_value = "1.0")]
  pub confidence: f64,

  /// Human-readable reasoning for this classification
  #[arg(long, default_value = "")]
  pub reasoning: String,

  /// TTL in days for this classification
  #[arg(long, default_value = "30")]
  pub ttl_days: i64,

  /// Create if not exists, update if already classified (keyed by domain).
  /// This flag is required to make the upsert intent explicit.
  #[arg(long)]
  pub ensure: bool,
}

#[derive(Serialize)]
struct ClassifyRequest {
  domain: String,
  classification_type: String,
  is_matching_site: bool,
  confidence: f64,
  reasoning: String,
  ttl_days: i64,
}

pub async fn execute(
  args: DomainArgs,
  client: &reqwest::Client,
  admin_url: &str,
) -> Result<(), CliError> {
  match args.command {
    DomainCommands::Classify(classify_args) => {
      classify(classify_args, client, admin_url).await
    }
  }
}

async fn classify(
  args: ClassifyArgs,
  client: &reqwest::Client,
  admin_url: &str,
) -> Result<(), CliError> {
  if !args.ensure {
    return Err(CliError::EnsureRequired);
  }

  let url = format!("{}/classify", admin_url);

  info!(
    "Classifying '{}' as '{}' (matching={}, confidence={})",
    args.domain,
    args.classification_type,
    args.is_matching_site,
    args.confidence
  );

  let response = client
    .post(&url)
    .json(&ClassifyRequest {
      domain: args.domain.clone(),
      classification_type: args.classification_type.clone(),
      is_matching_site: args.is_matching_site,
      confidence: args.confidence,
      reasoning: args.reasoning.clone(),
      ttl_days: args.ttl_days,
    })
    .send()
    .await?;

  let status = response.status();
  let body = response.text().await?;

  if status.is_success() {
    println!("{}", body.trim());
    Ok(())
  } else {
    Err(CliError::Api {
      status: status.as_u16(),
      body,
    })
  }
}
