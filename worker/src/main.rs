use clap::Parser;
use dns_smart_block_worker::{
  classify_with_llm,
  cli_args::CliArgs,
  error::WorkerError,
  web_classify::{extract_metadata, fetch_domain},
};
use tracing::{error, info, warn};

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

  let prompt_template = std::fs::read_to_string(&args.prompt_template)
    .map_err(|e| {
      error!(
        "Failed to read prompt template from {:?}: {}",
        args.prompt_template, e
      );
      e
    })?;

  let (html, status) =
    fetch_domain(&args.domain, args.http_timeout_sec, args.http_max_kb).await?;

  let metadata = extract_metadata(&args.domain, &html, status)?;

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
  info!("Is gaming site: {}", classification.is_matching_site);
  info!("Confidence: {:.2}", classification.confidence);

  Ok(())
}
