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
  /// Apply an admin classification for a domain or pattern (manual, never reconcile-expired)
  Classify(ClassifyArgs),
  /// Reconcile the provisioned classification set from a JSON file
  Reconcile(ReconcileArgs),
}

#[derive(Args, Debug)]
#[group(multiple = false)]
pub struct ClassifyTarget {
  /// Exact domain to classify (e.g. "example.com").
  /// Mutually exclusive with --pattern.
  #[arg(long)]
  pub domain: Option<String>,

  /// Regex pattern covering a family of domains (e.g. "^(.*\\.)?example\\.com$").
  /// Mutually exclusive with --domain.  The pattern is applied to all existing
  /// domains in the database and stored so future matches skip the LLM.
  #[arg(long)]
  pub pattern: Option<String>,
}

#[derive(Args, Debug)]
pub struct ClassifyArgs {
  #[command(flatten)]
  pub target: ClassifyTarget,

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
  /// Only meaningful for --domain; pattern rules never expire.
  #[arg(long)]
  pub ttl_days: Option<i64>,

  /// Create if not exists, update if already classified (keyed by domain).
  /// This flag is required to make the upsert intent explicit.
  #[arg(long)]
  pub ensure: bool,
}

#[derive(Serialize)]
struct ClassifyRequest {
  #[serde(skip_serializing_if = "Option::is_none")]
  domain: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pattern: Option<String>,
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
  /// The file must be a JSON array of objects.  Each object must have exactly
  /// one of `domain` (string) or `pattern` (regex string), plus
  /// `classification_type`, `is_matching_site`, `confidence`, and optionally
  /// `reasoning`.
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

  let domain_count = entries.iter().filter(|e| e.domain.is_some()).count();
  let pattern_count = entries.iter().filter(|e| e.pattern.is_some()).count();
  info!(
    "Reconciling {} domain entry(ies) and {} pattern entry(ies) from {:?}",
    domain_count, pattern_count, args.file
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

  // Validate that exactly one of domain or pattern is provided.
  match (&args.target.domain, &args.target.pattern) {
    (None, None) => {
      return Err(CliError::Api {
        status: 0,
        body: "Either --domain or --pattern must be provided.".to_string(),
      });
    }
    (Some(_), Some(_)) => {
      return Err(CliError::Api {
        status: 0,
        body: "--domain and --pattern are mutually exclusive.".to_string(),
      });
    }
    _ => {}
  }

  let url = format!("{}/classify", admin_url);

  if let Some(ref domain) = args.target.domain {
    info!(
      "Classifying '{}' as '{}' (matching={}, confidence={})",
      domain, args.classification_type, args.is_matching_site, args.confidence
    );
  } else if let Some(ref pattern) = args.target.pattern {
    info!(
      "Applying pattern classification for '{}' as '{}' (matching={}, confidence={})",
      pattern, args.classification_type, args.is_matching_site, args.confidence
    );
  }

  let response = client
    .post(&url)
    .json(&ClassifyRequest {
      domain: args.target.domain.clone(),
      pattern: args.target.pattern.clone(),
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
