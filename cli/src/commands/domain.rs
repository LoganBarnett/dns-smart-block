use crate::error::CliError;
use clap::{Args, Subcommand};
use dns_smart_block_common::db::ProvisionedEntry;
use serde::Serialize;
use std::path::PathBuf;
use tracing::info;

#[derive(Args, Debug)]
pub struct DomainArgs {
  #[command(subcommand)]
  pub command: DomainCommands,
}

#[derive(Subcommand, Debug)]
pub enum DomainCommands {
  /// Apply an admin classification for a domain (manual, never reconcile-expired)
  Classify(ClassifyArgs),
  /// Reconcile the provisioned classification set from a JSON file
  Reconcile(ReconcileArgs),
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

  /// TTL in days.  Omit to create a classification that never expires.
  #[arg(long)]
  pub ttl_days: Option<i64>,

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
  #[serde(skip_serializing_if = "Option::is_none")]
  ttl_days: Option<i64>,
}

#[derive(Args, Debug)]
pub struct ReconcileArgs {
  /// Path to a JSON file containing the desired provisioned classifications.
  /// The file must be a JSON array of objects with fields: domain,
  /// classification_type, is_matching_site, confidence, reasoning (optional).
  #[arg(long)]
  pub file: PathBuf,
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
    DomainCommands::Reconcile(reconcile_args) => {
      reconcile(reconcile_args, client, admin_url).await
    }
  }
}

async fn reconcile(
  args: ReconcileArgs,
  client: &reqwest::Client,
  admin_url: &str,
) -> Result<(), CliError> {
  let contents =
    std::fs::read_to_string(&args.file).map_err(|e| CliError::Api {
      status: 0,
      body: format!("Failed to read {:?}: {}", args.file, e),
    })?;

  let entries: Vec<ProvisionedEntry> = serde_json::from_str(&contents)
    .map_err(|e| CliError::Api {
      status: 0,
      body: format!("Failed to parse {:?}: {}", args.file, e),
    })?;

  info!(
    "Reconciling {} provisioned classification(s) from {:?}",
    entries.len(),
    args.file
  );

  let url = format!("{}/reconcile", admin_url);
  let response = client.post(&url).json(&entries).send().await?;
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
