pub mod domain;

use crate::error::CliError;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Commands {
  /// Domain management commands
  Domain(domain::DomainArgs),
}

pub async fn execute(
  command: Commands,
  client: &reqwest::Client,
  admin_url: &str,
) -> Result<(), CliError> {
  match command {
    Commands::Domain(args) => domain::execute(args, client, admin_url).await,
  }
}
