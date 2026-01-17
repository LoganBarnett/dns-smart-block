use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
  name = "dns-smart-block-worker",
  version,
  about = "Worker for DNS smart blocking using LLM classification"
)]
pub struct CliArgs {
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

  /// HTTP timeout in seconds.
  #[arg(long, env = "HTTP_TIMEOUT_SEC", default_value = "5")]
  pub http_timeout_sec: u64,

  /// Maximum HTTP response size in KB.
  #[arg(long, env = "HTTP_MAX_KB", default_value = "200")]
  pub http_max_kb: usize,
}
