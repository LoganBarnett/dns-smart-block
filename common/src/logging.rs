use clap::Parser;
use std::env;
use tracing_subscriber::prelude::*;

#[derive(Parser, Debug, Clone)]
pub struct LoggingArgs {
  /// Enable ANSI color codes in logs (default: auto-detect TTY)
  #[arg(long, env = "LOG_ANSI")]
  pub log_ansi: Option<bool>,

  /// Include timestamps in logs (default: true unless running under systemd journal)
  #[arg(long, env = "LOG_TIMESTAMP")]
  pub log_timestamp: Option<bool>,
}

impl LoggingArgs {
  /// Initialize tracing subscriber with appropriate settings for the environment.
  /// When the journald socket is available, logs are sent there as structured
  /// fields.  Otherwise falls back to a plain stderr formatter.
  pub fn init_tracing(&self) {
    let should_use_ansi = self.should_use_ansi();
    let should_use_timestamp = self.should_use_timestamp();

    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
      .add_directive(tracing::Level::INFO.into());

    // Try to connect to the journald socket; fall back to stderr fmt if unavailable.
    let journald = tracing_journald::layer().ok();

    // Only emit fmt logs when journald is not active.
    let fmt_layer = if journald.is_none() {
      let layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(should_use_ansi);
      if should_use_timestamp {
        Some(layer.boxed())
      } else {
        Some(layer.without_time().boxed())
      }
    } else {
      None
    };

    tracing_subscriber::registry()
      .with(env_filter)
      .with(journald)
      .with(fmt_layer)
      .init();
  }

  /// Determine if ANSI colors should be used
  fn should_use_ansi(&self) -> bool {
    match self.log_ansi {
      Some(explicit) => explicit,
      None => {
        // Auto-detect: use ANSI if stderr is a TTY
        atty::is(atty::Stream::Stderr)
      }
    }
  }

  /// Determine if timestamps should be included
  fn should_use_timestamp(&self) -> bool {
    match self.log_timestamp {
      Some(explicit) => explicit,
      None => {
        // Auto-detect: disable timestamps if running under systemd journal
        !Self::is_journald_context()
      }
    }
  }

  /// Check if we're running under systemd journal
  fn is_journald_context() -> bool {
    // systemd sets JOURNAL_STREAM when using journal for stdout/stderr
    env::var("JOURNAL_STREAM").is_ok()
  }
}
