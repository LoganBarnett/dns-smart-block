use clap::Parser;
use dns_smart_block_common::logging::LoggingArgs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "dns-smart-block-blocklist-server")]
#[command(about = "Serves DNS blocklists from database classifications")]
pub struct CliArgs {
  #[command(flatten)]
  pub logging: LoggingArgs,

  /// PostgreSQL connection URL (without password if using password file)
  #[arg(long, env = "DATABASE_URL")]
  pub database_url: String,

  /// Path to file containing database password
  #[arg(long, env = "DATABASE_PASSWORD_FILE")]
  pub database_password_file: Option<PathBuf>,

  /// Address to listen on for the public server (blocklist, metrics, health):
  /// host:port for TCP, /path/to.sock for Unix socket, or sd-listen for
  /// systemd socket activation.
  #[arg(long, env = "PUBLIC_LISTEN", default_value = "0.0.0.0:3000")]
  pub public_listen: String,

  /// Address to listen on for the admin server (classifications, reprojection):
  /// host:port for TCP, /path/to.sock for Unix socket, or sd-listen for
  /// systemd socket activation.
  #[arg(long, env = "ADMIN_LISTEN", default_value = "127.0.0.1:8080")]
  pub admin_listen: String,

  /// NATS server URL for requeueing errored domains (optional)
  #[arg(long, env = "NATS_URL")]
  pub nats_url: Option<String>,

  /// NATS subject for the domain queue
  #[arg(long, env = "NATS_SUBJECT", default_value = "dns.domains")]
  pub nats_subject: String,
}
