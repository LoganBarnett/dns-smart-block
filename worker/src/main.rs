use clap::Parser;
use reqwest::redirect::Policy;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tracing::{error, info, warn};

#[derive(Error, Debug)]
enum WorkerError {
  #[error("HTTP fetch failed: {0}")]
  HttpError(#[from] reqwest::Error),

  #[error("IO error: {0}")]
  IoError(#[from] std::io::Error),

  #[error("JSON error: {0}")]
  JsonError(#[from] serde_json::Error),
}

#[derive(Parser, Debug)]
#[command(
  name = "dns-smart-block-worker",
  version,
  about = "Worker for DNS smart blocking using LLM classification"
)]
struct Args {
  /// Domain to classify
  #[arg(long, env = "DOMAIN")]
  domain: String,

  /// Ollama API URL
  #[arg(
    long,
    env = "OLLAMA_URL",
    default_value = "http://localhost:11434"
  )]
  ollama_url: String,

  /// Ollama model to use
  #[arg(
    long,
    env = "OLLAMA_MODEL",
    default_value = "llama3.1:8b-instruct-q4_K_M"
  )]
  ollama_model: String,

  /// Path to prompt template file
  #[arg(
    long,
    env = "PROMPT_TEMPLATE",
    default_value = "prompt-template.txt"
  )]
  prompt_template: PathBuf,

  /// HTTP timeout in seconds
  #[arg(long, env = "HTTP_TIMEOUT_SEC", default_value = "5")]
  http_timeout_sec: u64,

  /// Maximum HTTP response size in KB
  #[arg(long, env = "HTTP_MAX_KB", default_value = "200")]
  http_max_kb: usize,
}

#[derive(Serialize, Debug)]
struct SiteMetadata {
  domain: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  title: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  og_title: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  og_description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  og_site_name: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  language: Option<String>,
  http_status: u16,
}

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

async fn fetch_domain(
  domain: &str,
  timeout_sec: u64,
  max_kb: usize,
) -> Result<(String, u16), WorkerError> {
  info!("Fetching domain: {}", domain);

  let client = reqwest::Client::builder()
    .redirect(Policy::limited(10))
    .timeout(Duration::from_secs(timeout_sec))
    .user_agent(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
       AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 \
       Safari/605.1.15",
    )
    .gzip(true)
    .build()?;

  let url = if domain.starts_with("http://")
    || domain.starts_with("https://")
  {
    domain.to_string()
  } else {
    format!("https://{}", domain)
  };

  let response = client
    .get(&url)
    .header("Accept", "text/html,application/xhtml+xml,\
      application/xml;q=0.9,*/*;q=0.8")
    .header("Accept-Language", "en-US,en;q=0.9")
    .send()
    .await?;

  let status = response.status().as_u16();
  info!("HTTP status: {}", status);

  let max_bytes = max_kb * 1024;
  let body_bytes = response.bytes().await?;

  let body = if body_bytes.len() > max_bytes {
    info!(
      "Truncating response from {} bytes to {} KB",
      body_bytes.len(),
      max_kb
    );
    &body_bytes[..max_bytes]
  } else {
    &body_bytes[..]
  };

  let html = String::from_utf8_lossy(body).to_string();
  Ok((html, status))
}

fn extract_metadata(
  domain: &str,
  html: &str,
  status: u16,
) -> Result<SiteMetadata, WorkerError> {
  info!("Extracting metadata from HTML");

  let document = Html::parse_document(html);

  // Extract title
  let title = Selector::parse("title")
    .ok()
    .and_then(|sel| document.select(&sel).next())
    .map(|el| el.text().collect::<String>().trim().to_string())
    .filter(|s| !s.is_empty());

  // Extract meta description
  let description =
    Selector::parse("meta[name='description']")
      .ok()
      .and_then(|sel| document.select(&sel).next())
      .and_then(|el| el.value().attr("content"))
      .map(|s| s.trim().to_string())
      .filter(|s| !s.is_empty());

  // Extract og:title
  let og_title = Selector::parse("meta[property='og:title']")
    .ok()
    .and_then(|sel| document.select(&sel).next())
    .and_then(|el| el.value().attr("content"))
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());

  // Extract og:description
  let og_description =
    Selector::parse("meta[property='og:description']")
      .ok()
      .and_then(|sel| document.select(&sel).next())
      .and_then(|el| el.value().attr("content"))
      .map(|s| s.trim().to_string())
      .filter(|s| !s.is_empty());

  // Extract og:site_name
  let og_site_name =
    Selector::parse("meta[property='og:site_name']")
      .ok()
      .and_then(|sel| document.select(&sel).next())
      .and_then(|el| el.value().attr("content"))
      .map(|s| s.trim().to_string())
      .filter(|s| !s.is_empty());

  // Extract language from html tag
  let language = Selector::parse("html")
    .ok()
    .and_then(|sel| document.select(&sel).next())
    .and_then(|el| el.value().attr("lang"))
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());

  Ok(SiteMetadata {
    domain: domain.to_string(),
    title,
    description,
    og_title,
    og_description,
    og_site_name,
    language,
    http_status: status,
  })
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

  let args = Args::parse();

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
