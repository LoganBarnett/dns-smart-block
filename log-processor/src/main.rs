use clap::Parser;
use dns_smart_block_log_processor::{
  ProcessorError, Result, cli_args::CliArgs, log_parser::LogParser,
  log_source::LogSource, queue::QueuePublisher,
};
use futures::StreamExt;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
  let args = CliArgs::parse();

  // Initialize logging with auto-detection and CLI overrides
  args.logging.init_tracing();

  info!("Starting DNS Smart Block Log Processor");
  info!("Log source: {}", args.log_source);
  info!("Domain pattern: {}", args.domain_pattern);
  info!("Capture group: {}", args.domain_capture_group);
  if let Some(ref filter) = args.line_filter {
    info!("Line filter: {}", filter);
  }
  info!("NATS URL: {}", args.nats_url);
  info!("NATS subject: {}", args.nats_subject);

  // Initialize components
  let parser = LogParser::new(
    &args.domain_pattern,
    args.domain_capture_group,
    args.line_filter.as_deref(),
  )?;
  let queue =
    QueuePublisher::new(&args.nats_url, args.nats_subject.clone()).await?;

  // Create log source
  let log_source = if args.is_command_source() {
    let cmd = args.get_command().ok_or_else(|| {
      ProcessorError::InvalidLogSource("Invalid command".to_string())
    })?;
    LogSource::from_command(cmd)
  } else {
    let path = args.get_file_path().ok_or_else(|| {
      ProcessorError::InvalidLogSource("Invalid file path".to_string())
    })?;
    LogSource::from_file(path)
  };

  info!("Starting log stream processing");

  let mut stream = log_source.into_stream().await?;

  while let Some(line_result) = stream.next().await {
    match line_result {
      Ok(line) => {
        if let Some(domain) = parser.parse_log_line(&line) {
          info!("Found domain in log: {}", domain);

          match queue.publish_domain(&domain).await {
            Ok(()) => {
              info!("Queued domain: {}", domain);
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
