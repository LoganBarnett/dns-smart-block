mod cli_args;
mod error;
mod web_classify;
use clap::Parser;
use crate::{
  cli_args::CliArgs,
  error::WorkerError,
  web_classify::{SiteMetadata, extract_metadata, fetch_domain},
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

#[derive(Serialize, Debug)]
struct OllamaRequest {
  model: String,
  prompt: String,
  format: String,
  stream: bool,
}

#[derive(Deserialize, Debug)]
struct OllamaResponse {
  response: String,
}

#[derive(Deserialize, Debug)]
struct ClassificationResult {
  is_matching_site: bool,
  confidence: f64,
}

async fn classify_with_llm(
  metadata: &SiteMetadata,
  ollama_url: &str,
  model: &str,
  prompt_template: &str,
) -> Result<ClassificationResult, WorkerError> {
  info!("Classifying domain with LLM");

  let metadata_json = serde_json::to_string(metadata)?;
  let prompt = prompt_template.replace(
    "{{INPUT_JSON}}",
    &metadata_json,
  );

  let ollama_request = OllamaRequest {
    model: model.to_string(),
    prompt,
    format: "json".to_string(),
    stream: false,
  };

  let client = reqwest::Client::new();
  let response = client
    .post(format!("{}/api/generate", ollama_url))
    .json(&ollama_request)
    .send()
    .await?;

  if !response.status().is_success() {
    error!(
      "Ollama API returned error: {}",
      response.status()
    );
    return Err(WorkerError::HttpError(
      response.error_for_status().unwrap_err(),
    ));
  }

  let ollama_response: OllamaResponse = response.json().await?;
  info!("LLM response: {}", ollama_response.response);

  let classification: ClassificationResult =
    serde_json::from_str(&ollama_response.response)?;

  Ok(classification)
}

#[tokio::main]
async fn main() -> Result<(), WorkerError> {
  tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
    .with_env_filter(
      tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into()),
    )
    .init();

  let args = CliArgs::parse();

  info!("Starting DNS Smart Block Worker");
  info!("Domain: {}", args.domain);
  info!("Ollama URL: {}", args.ollama_url);
  info!("Ollama Model: {}", args.ollama_model);

  let prompt_template = std::fs::read_to_string(
    &args.prompt_template,
  )
  .map_err(|e| {
    error!(
      "Failed to read prompt template from {:?}: {}",
      args.prompt_template, e
    );
    e
  })?;

  let (html, status) = fetch_domain(
    &args.domain,
    args.http_timeout_sec,
    args.http_max_kb,
  )
  .await?;

  let metadata =
    extract_metadata(&args.domain, &html, status)?;

  if metadata.title.is_none()
    && metadata.description.is_none()
    && metadata.og_title.is_none()
    && metadata.og_description.is_none()
  {
    warn!("No useful metadata found for domain");
  }

  info!("Metadata: {:#?}", metadata);

  let classification = classify_with_llm(
    &metadata,
    &args.ollama_url,
    &args.ollama_model,
    &prompt_template,
  )
  .await?;

  info!("Classification result: {:#?}", classification);
  info!(
    "Is gaming site: {}",
    classification.is_matching_site
  );
  info!("Confidence: {:.2}", classification.confidence);

  Ok(())
}
