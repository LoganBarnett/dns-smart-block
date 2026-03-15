mod cli;
mod db;
mod metrics;
mod server;

use clap::Parser;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args = cli::CliArgs::parse();
  args.logging.init_tracing();
  server::run(args).await
}
