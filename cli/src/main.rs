mod commands;
mod error;

use clap::Parser;
use dns_smart_block_common::logging::LoggingArgs;
use error::CliError;
use tracing::error;

#[derive(Parser, Debug)]
#[command(name = "dns-smart-block-cli")]
#[command(about = "CLI for managing DNS Smart Block")]
struct Cli {
  #[command(flatten)]
  logging: LoggingArgs,

  /// Base URL for the blocklist server admin API
  #[arg(long, env = "ADMIN_URL", default_value = "http://127.0.0.1:8080")]
  admin_url: String,

  #[command(subcommand)]
  command: commands::Commands,
}

#[tokio::main]
async fn main() {
  let cli = Cli::parse();
  cli.logging.init_tracing();

  if let Err(e) = run(cli).await {
    error!("{}", e);
    std::process::exit(1);
  }
}

async fn run(cli: Cli) -> Result<(), CliError> {
  let client = reqwest::Client::new();
  commands::execute(cli.command, &client, &cli.admin_url).await
}
