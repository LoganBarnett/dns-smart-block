use clap::Parser;
use dns_smart_block_log_processor::{
  cli_args::CliArgs,
  dnsdist::DnsdistClient,
  log_parser::LogParser,
  log_source::LogSource,
  queue::QueuePublisher,
  ProcessorError, Result,
};
use futures::StreamExt;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

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

  info!("Starting DNS Smart Block Log Processor");
  info!("Log source: {}", args.log_source);
  info!("NATS URL: {}", args.nats_url);
  info!("NATS subject: {}", args.nats_subject);

  // Initialize components
  let parser = LogParser::new()?;
  let queue = QueuePublisher::new(&args.nats_url, args.nats_subject.clone()).await?;

  let dnsdist_client = if let Some(ref url) = args.dnsdist_api_url {
    if args.skip_dnsdist_check {
      info!("dnsdist API URL provided but checks are disabled");
      None
    } else {
      info!("Initializing dnsdist client with URL: {}", url);
      Some(DnsdistClient::new(
        url.clone(),
        args.dnsdist_api_key.clone(),
      ))
    }
  } else {
    info!("No dnsdist API URL provided, will queue all domains");
    None
  };

  // Track seen domains to avoid duplicate processing
  let seen_domains: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

  // Create log source
  let log_source = if args.is_command_source() {
    let cmd = args
      .get_command()
      .ok_or_else(|| ProcessorError::InvalidLogSource("Invalid command".to_string()))?;
    LogSource::from_command(cmd)
  } else {
    let path = args
      .get_file_path()
      .ok_or_else(|| ProcessorError::InvalidLogSource("Invalid file path".to_string()))?;
    LogSource::from_file(path)
  };

  info!("Starting log stream processing");

  let mut stream = log_source.into_stream().await?;

  while let Some(line_result) = stream.next().await {
    match line_result {
      Ok(line) => {
        if let Some(domain) = parser.parse_log_line(&line) {
          // Check if we've already seen this domain
          let mut seen = seen_domains.lock().await;
          if seen.contains(&domain) {
            continue;
          }

          info!("Found domain in log: {}", domain);

          // Check if domain is already blocked in dnsdist (if configured)
          if let Some(ref client) = dnsdist_client {
            match client.is_domain_blocked(&domain).await {
              Ok(true) => {
                info!(
                  "Domain {} is already blocked, skipping queue",
                  domain
                );
                seen.insert(domain);
                continue;
              }
              Ok(false) => {
                info!("Domain {} is not blocked, adding to queue", domain);
              }
              Err(e) => {
                warn!(
                  "Failed to check if domain {} is blocked: {}. Will queue anyway.",
                  domain, e
                );
              }
            }
          }

          // Publish to queue
          match queue.publish_domain(&domain).await {
            Ok(()) => {
              info!("Successfully queued domain: {}", domain);
              seen.insert(domain);
            }
            Err(e) => {
              error!("Failed to publish domain {} to queue: {}", domain, e);
            }
          }
        }
      }
      Err(e) => {
        error!("Error reading log line: {}", e);
      }
    }
  }

  info!("Log stream ended");
  Ok(())
}
