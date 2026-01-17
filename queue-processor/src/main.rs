use clap::Parser;
use dns_smart_block_worker::{
  classify_with_llm,
  web_classify::{extract_metadata, fetch_domain},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "dns-smart-block-queue-processor")]
#[command(about = "Processes domains from NATS queue and classifies them")]
struct CliArgs {
  /// NATS server URL
  #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
  nats_url: String,

  /// NATS subject to subscribe to
  #[arg(long, env = "NATS_SUBJECT", default_value = "dns.domains")]
  nats_subject: String,

  /// Ollama server URL
  #[arg(long, env = "OLLAMA_URL", default_value = "http://localhost:11434")]
  ollama_url: String,

  /// Ollama model to use
  #[arg(long, env = "OLLAMA_MODEL", default_value = "llama2")]
  ollama_model: String,

  /// Path to prompt template file
  #[arg(long, env = "PROMPT_TEMPLATE")]
  prompt_template: PathBuf,

  /// HTTP timeout in seconds for fetching domains
  #[arg(long, env = "HTTP_TIMEOUT_SEC", default_value = "10")]
  http_timeout_sec: u64,

  /// Maximum KB to download from each domain
  #[arg(long, env = "HTTP_MAX_KB", default_value = "100")]
  http_max_kb: usize,

  /// dnsdist API URL for blocking domains
  #[arg(long, env = "DNSDIST_API_URL")]
  dnsdist_api_url: Option<String>,

  /// dnsdist API key
  #[arg(long, env = "DNSDIST_API_KEY")]
  dnsdist_api_key: Option<String>,

  /// Minimum confidence threshold to block (0.0 to 1.0)
  #[arg(long, env = "MIN_CONFIDENCE", default_value = "0.8")]
  min_confidence: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DomainMessage {
  domain: String,
  timestamp: i64,
}

#[derive(Error, Debug)]
enum ProcessorError {
  #[error("NATS error: {0}")]
  NatsError(String),

  #[error("Worker error: {0}")]
  WorkerError(#[from] dns_smart_block_worker::error::WorkerError),

  #[error("IO error: {0}")]
  IoError(#[from] std::io::Error),

  #[error("JSON error: {0}")]
  JsonError(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, ProcessorError>;

async fn process_domain(
  domain: &str,
  args: &CliArgs,
  prompt_template: &str,
) -> Result<()> {
  info!("Processing domain: {}", domain);

  // Fetch domain content
  let (html, status) =
    match fetch_domain(domain, args.http_timeout_sec, args.http_max_kb).await {
      Ok(result) => result,
      Err(e) => {
        warn!("Failed to fetch domain {}: {}", domain, e);
        return Ok(()); // Don't fail the whole process, just skip this domain
      }
    };

  info!("Fetched domain {} with status {}", domain, status);

  // Extract metadata
  let metadata = match extract_metadata(domain, &html, status) {
    Ok(meta) => meta,
    Err(e) => {
      warn!("Failed to extract metadata from {}: {}", domain, e);
      return Ok(());
    }
  };

  if metadata.title.is_none()
    && metadata.description.is_none()
    && metadata.og_title.is_none()
    && metadata.og_description.is_none()
  {
    warn!("No useful metadata found for domain {}", domain);
    return Ok(());
  }

  info!("Extracted metadata from {}: {:#?}", domain, metadata);

  // Classify with LLM
  let classification = match classify_with_llm(
    &metadata,
    &args.ollama_url,
    &args.ollama_model,
    prompt_template,
  )
  .await
  {
    Ok(result) => result,
    Err(e) => {
      error!("Failed to classify domain {}: {}", domain, e);
      return Ok(());
    }
  };

  info!(
    "Classification result for {}: matching={}, confidence={:.2}",
    domain, classification.is_matching_site, classification.confidence
  );

  // Check if we should block this domain
  if classification.is_matching_site
    && classification.confidence >= args.min_confidence
  {
    info!(
      "Domain {} matches criteria with confidence {:.2}",
      domain, classification.confidence
    );

    // TODO: Send block command to dnsdist
    if let Some(ref dnsdist_url) = args.dnsdist_api_url {
      info!(
        "Would block domain {} via dnsdist API at {}",
        domain, dnsdist_url
      );
      // Future: implement dnsdist blocking API call
    } else {
      info!(
        "No dnsdist API configured, skipping block action for {}",
        domain
      );
    }
  } else {
    info!(
      "Domain {} does not match criteria (matching={}, confidence={:.2})",
      domain, classification.is_matching_site, classification.confidence
    );
  }

  Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
    .with_env_filter(
      tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into()),
    )
    .init();

  let args = CliArgs::parse();

  info!("Starting DNS Smart Block Queue Processor");
  info!("NATS URL: {}", args.nats_url);
  info!("NATS subject: {}", args.nats_subject);
  info!("Ollama URL: {}", args.ollama_url);
  info!("Ollama model: {}", args.ollama_model);

  // Load prompt template
  let prompt_template = std::fs::read_to_string(&args.prompt_template)
    .map_err(|e| {
      error!(
        "Failed to read prompt template from {:?}: {}",
        args.prompt_template, e
      );
      e
    })?;

  info!("Loaded prompt template from {:?}", args.prompt_template);

  // Connect to NATS
  info!("Connecting to NATS...");
  let client = async_nats::connect(&args.nats_url)
    .await
    .map_err(|e| ProcessorError::NatsError(e.to_string()))?;

  info!("Connected to NATS successfully");

  // Subscribe to domain messages
  info!("Subscribing to subject: {}", args.nats_subject);
  let mut subscriber = client
    .subscribe(args.nats_subject.clone())
    .await
    .map_err(|e| ProcessorError::NatsError(e.to_string()))?;

  info!("Subscribed successfully, waiting for messages...");

  // Process messages
  while let Some(message) = subscriber.next().await {
    let payload = message.payload;

    // Deserialize domain message
    match serde_json::from_slice::<DomainMessage>(&payload) {
      Ok(domain_msg) => {
        info!(
          "Received domain: {} (timestamp: {})",
          domain_msg.domain, domain_msg.timestamp
        );

        // Process the domain
        if let Err(e) =
          process_domain(&domain_msg.domain, &args, &prompt_template).await
        {
          error!("Error processing domain {}: {}", domain_msg.domain, e);
        }
      }
      Err(e) => {
        error!("Failed to deserialize message: {}", e);
        warn!("Raw payload: {:?}", String::from_utf8_lossy(&payload));
      }
    }
  }

  info!("NATS subscription ended");
  Ok(())
}
