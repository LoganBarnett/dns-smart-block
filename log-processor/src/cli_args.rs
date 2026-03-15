use clap::Parser;
use dns_smart_block_common::logging::LoggingArgs;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "dns-smart-block-log-processor")]
#[command(about = "Watches DNS logs and queues domains for classification")]
pub struct CliArgs {
  #[command(flatten)]
  pub logging: LoggingArgs,

  /// Log source: either a file path or a command to run (prefix with 'cmd:').
  /// Examples: '/var/log/dns.log' or 'cmd:journalctl --follow --unit=blocky.service'
  #[arg(long, env = "LOG_SOURCE")]
  pub log_source: String,

  /// Regex pattern to extract the domain from a log line.  Use a capture group
  /// to mark the domain portion; see --domain-capture-group.
  /// Example for Blocky: 'question_name=(\w(?:[\w-]*\w)?(?:\.\w(?:[\w-]*\w)?)+)\.'
  #[arg(long, env = "DOMAIN_PATTERN")]
  pub domain_pattern: String,

  /// Which capture group in --domain-pattern contains the domain (1-indexed).
  #[arg(long, env = "DOMAIN_CAPTURE_GROUP", default_value = "1")]
  pub domain_capture_group: usize,

  /// Optional regex; when set, only log lines matching this pattern are
  /// considered for domain extraction.
  /// Example for Blocky: 'response_type=RESOLVED'
  #[arg(long, env = "LINE_FILTER")]
  pub line_filter: Option<String>,

  /// NATS server URL
  #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
  pub nats_url: String,

  /// NATS subject/topic to publish domains to
  #[arg(long, env = "NATS_SUBJECT", default_value = "dns.domains")]
  pub nats_subject: String,
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
