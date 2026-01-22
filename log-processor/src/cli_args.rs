use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "dns-smart-block-log-processor")]
#[command(about = "Watches DNS logs and queues domains for classification")]
pub struct CliArgs {
  /// Log source: either a file path or a command to run (prefix with 'cmd:')
  /// Examples: '/var/log/dnsdist.log' or 'cmd:journalctl -f -u dnsdist'
  #[arg(long, env = "LOG_SOURCE")]
  pub log_source: String,

  /// NATS server URL
  #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
  pub nats_url: String,

  /// NATS subject/topic to publish domains to
  #[arg(long, env = "NATS_SUBJECT", default_value = "dns.domains")]
  pub nats_subject: String,

  /// PostgreSQL connection URL (without password if using password file)
  #[arg(long, env = "DATABASE_URL")]
  pub database_url: String,

  /// Path to file containing database password
  #[arg(long, env = "DATABASE_PASSWORD_FILE")]
  pub database_password_file: Option<PathBuf>,

  /// dnsdist API URL (to check if domain is already blocked)
  #[arg(long, env = "DNSDIST_API_URL")]
  pub dnsdist_api_url: Option<String>,

  /// dnsdist API key for authentication
  #[arg(long, env = "DNSDIST_API_KEY")]
  pub dnsdist_api_key: Option<String>,

  /// Skip dnsdist check (always queue domains even if potentially blocked)
  #[arg(long, env = "SKIP_DNSDIST_CHECK", default_value = "false")]
  pub skip_dnsdist_check: bool,
}

impl CliArgs {
  pub fn is_command_source(&self) -> bool {
    self.log_source.starts_with("cmd:")
  }

  pub fn get_command(&self) -> Option<Vec<String>> {
    if self.is_command_source() {
      let cmd = self.log_source.strip_prefix("cmd:")?.trim();
      Some(cmd.split_whitespace().map(|s| s.to_string()).collect())
    } else {
      None
    }
  }

  pub fn get_file_path(&self) -> Option<PathBuf> {
    if !self.is_command_source() {
      Some(PathBuf::from(&self.log_source))
    } else {
      None
    }
  }
}
