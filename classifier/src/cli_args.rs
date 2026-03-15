use clap::Parser;
use dns_smart_block_common::logging::LoggingArgs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
  name = "dns-smart-block-classifier",
  version,
  about = "Classifier for DNS smart blocking using LLM classification"
)]
pub struct CliArgs {
  #[command(flatten)]
  pub logging: LoggingArgs,

  /// Domain to classify.
  #[arg(long, env = "DOMAIN")]
  pub domain: String,

  /// Ollama API URL.
  #[arg(long, env = "OLLAMA_URL", default_value = "http://localhost:11434")]
  pub ollama_url: String,

  /// Ollama model to use.
  #[arg(
    long,
    env = "OLLAMA_MODEL",
    default_value = "llama3.1:8b-instruct-q4_K_M"
  )]
  pub ollama_model: String,

  /// Path to prompt template file.
  #[arg(long, env = "PROMPT_TEMPLATE", default_value = "prompt-template.txt")]
  pub prompt_template: PathBuf,

  /// Classification type (e.g., "gaming").
  #[arg(long, env = "CLASSIFICATION_TYPE", default_value = "gaming")]
  pub classification_type: String,

  /// HTTP timeout in seconds.
  #[arg(long, env = "HTTP_TIMEOUT_SEC", default_value = "5")]
  pub http_timeout_sec: u64,

  /// Maximum HTTP response size in KB.
  #[arg(long, env = "HTTP_MAX_KB", default_value = "200")]
  pub http_max_kb: usize,

  /// Pre-resolved IP address for the domain.  When provided, the HTTP fetch
  /// connects directly to this IP instead of resolving the domain through the
  /// local DNS stack, avoiding a duplicate log entry in the upstream resolver.
  /// The Host header and TLS SNI still use the domain name.
  #[arg(long, env = "RESOLVED_IP")]
  pub resolved_ip: Option<String>,

  /// Output format (json or human-readable).
  #[arg(long, env = "OUTPUT", default_value = "human")]
  pub output: String,
}
