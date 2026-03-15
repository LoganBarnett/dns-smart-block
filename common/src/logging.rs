use clap::Parser;
use std::env;

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
    /// Initialize tracing subscriber with appropriate settings for the environment
    pub fn init_tracing(&self) {
        let should_use_ansi = self.should_use_ansi();
        let should_use_timestamp = self.should_use_timestamp();

        let fmt = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_ansi(should_use_ansi)
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(tracing::Level::INFO.into()),
            );

        if should_use_timestamp {
            fmt.init();
        } else {
            fmt.without_time().init();
        }
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
