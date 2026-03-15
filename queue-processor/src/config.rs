use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
  #[error("Failed to read config file: {0}")]
  IoError(#[from] std::io::Error),

  #[error("Failed to parse TOML config: {0}")]
  TomlError(#[from] toml::de::Error),

  #[error("Configuration validation failed: {0}")]
  ValidationError(String),
}

/// Global Ollama LLM configuration shared by all classifiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
  /// Ollama server URL (e.g., "http://localhost:11434")
  pub url: String,

  /// Default Ollama model to use for classification (e.g., "llama3.2:3b")
  pub model: String,
}

/// Global HTTP fetching configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
  /// HTTP timeout for fetching domains (seconds)
  #[serde(default = "default_http_timeout_sec")]
  pub timeout_sec: u64,

  /// Maximum KB to download from each domain
  #[serde(default = "default_http_max_kb")]
  pub max_kb: usize,
}

fn default_http_timeout_sec() -> u64 {
  120
}

fn default_http_max_kb() -> usize {
  100
}

/// Global classification defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
  /// Default minimum confidence threshold to block (0.0 to 1.0)
  #[serde(default = "default_min_confidence")]
  pub min_confidence: f64,

  /// Default TTL for classifications in days
  #[serde(default = "default_ttl_days")]
  pub ttl_days: i64,
}

fn default_min_confidence() -> f64 {
  0.8
}

fn default_ttl_days() -> i64 {
  10
}

/// Individual classifier configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierConfig {
  /// Classifier name (used as classification_type in database)
  pub name: String,

  /// Path to prompt template file
  pub prompt_template: PathBuf,

  /// Override Ollama model for this classifier (optional)
  pub ollama_model: Option<String>,

  /// Override minimum confidence for this classifier (optional)
  pub min_confidence: Option<f64>,

  /// Override TTL for this classifier (optional)
  pub ttl_days: Option<i64>,

  /// Override HTTP timeout for this classifier (optional)
  pub http_timeout_sec: Option<u64>,

  /// Override HTTP max KB for this classifier (optional)
  pub http_max_kb: Option<usize>,
}

impl ClassifierConfig {
  /// Get the effective Ollama model for this classifier.
  pub fn effective_ollama_model(&self, global: &OllamaConfig) -> String {
    self
      .ollama_model
      .clone()
      .unwrap_or_else(|| global.model.clone())
  }

  /// Get the effective minimum confidence for this classifier.
  pub fn effective_min_confidence(&self, defaults: &DefaultsConfig) -> f64 {
    self.min_confidence.unwrap_or(defaults.min_confidence)
  }

  /// Get the effective TTL for this classifier.
  pub fn effective_ttl_days(&self, defaults: &DefaultsConfig) -> i64 {
    self.ttl_days.unwrap_or(defaults.ttl_days)
  }

  /// Get the effective HTTP timeout for this classifier.
  pub fn effective_http_timeout_sec(&self, http: &HttpConfig) -> u64 {
    self.http_timeout_sec.unwrap_or(http.timeout_sec)
  }

  /// Get the effective HTTP max KB for this classifier.
  pub fn effective_http_max_kb(&self, http: &HttpConfig) -> usize {
    self.http_max_kb.unwrap_or(http.max_kb)
  }
}

/// Main configuration file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
  /// Global Ollama configuration
  pub ollama: OllamaConfig,

  /// Global HTTP fetching configuration
  #[serde(default)]
  pub http: HttpConfig,

  /// Global classification defaults
  #[serde(default)]
  pub defaults: DefaultsConfig,

  /// List of classifiers to run on each domain
  #[serde(rename = "classifier", default)]
  pub classifiers: Vec<ClassifierConfig>,
}

impl Default for HttpConfig {
  fn default() -> Self {
    Self {
      timeout_sec: default_http_timeout_sec(),
      max_kb: default_http_max_kb(),
    }
  }
}

impl Default for DefaultsConfig {
  fn default() -> Self {
    Self {
      min_confidence: default_min_confidence(),
      ttl_days: default_ttl_days(),
    }
  }
}

impl Config {
  /// Load configuration from a TOML file.
  pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    config.validate()?;
    Ok(config)
  }

  /// Validate the configuration.
  fn validate(&self) -> Result<(), ConfigError> {
    // Ensure at least one classifier is configured.
    if self.classifiers.is_empty() {
      return Err(ConfigError::ValidationError(
        "No classifiers configured. At least one classifier is required."
          .to_string(),
      ));
    }

    // Validate each classifier.
    for classifier in &self.classifiers {
      // Check that name is not empty.
      if classifier.name.trim().is_empty() {
        return Err(ConfigError::ValidationError(
          "Classifier name cannot be empty".to_string(),
        ));
      }

      // Check that prompt template file exists.
      if !classifier.prompt_template.exists() {
        return Err(ConfigError::ValidationError(format!(
          "Prompt template file does not exist: {}",
          classifier.prompt_template.display()
        )));
      }

      // Validate min_confidence if specified.
      if let Some(conf) = classifier.min_confidence {
        if !(0.0..=1.0).contains(&conf) {
          return Err(ConfigError::ValidationError(format!(
            "Classifier '{}': min_confidence must be between 0.0 and 1.0, got {}",
            classifier.name, conf
          )));
        }
      }

      // Validate TTL if specified.
      if let Some(ttl) = classifier.ttl_days {
        if ttl < 0 {
          return Err(ConfigError::ValidationError(format!(
            "Classifier '{}': ttl_days must be non-negative, got {}",
            classifier.name, ttl
          )));
        }
      }
    }

    // Validate global defaults.
    if !(0.0..=1.0).contains(&self.defaults.min_confidence) {
      return Err(ConfigError::ValidationError(format!(
        "Global min_confidence must be between 0.0 and 1.0, got {}",
        self.defaults.min_confidence
      )));
    }

    if self.defaults.ttl_days < 0 {
      return Err(ConfigError::ValidationError(format!(
        "Global ttl_days must be non-negative, got {}",
        self.defaults.ttl_days
      )));
    }

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::NamedTempFile;

  #[test]
  fn test_parse_valid_config() {
    let gaming = NamedTempFile::new().unwrap();
    let video = NamedTempFile::new().unwrap();

    let config_content = format!(
      r#"
[ollama]
url = "http://localhost:11434"
model = "llama3.2:3b"

[http]
timeout_sec = 120
max_kb = 100

[defaults]
min_confidence = 0.8
ttl_days = 10

[[classifier]]
name = "gaming"
prompt_template = "{}"

[[classifier]]
name = "video-streaming"
prompt_template = "{}"
min_confidence = 0.9
"#,
      gaming.path().display(),
      video.path().display()
    );

    let config: Config = toml::from_str(&config_content).unwrap();
    assert_eq!(config.classifiers.len(), 2);
    assert_eq!(config.classifiers[0].name, "gaming");
    assert_eq!(config.classifiers[1].name, "video-streaming");
    assert_eq!(config.classifiers[1].min_confidence, Some(0.9));
  }

  #[test]
  fn test_empty_classifiers_validation_error() {
    let config_content = r#"
[ollama]
url = "http://localhost:11434"
model = "llama3.2:3b"
"#;

    let config: Config = toml::from_str(config_content).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    assert!(
      result
        .unwrap_err()
        .to_string()
        .contains("No classifiers configured")
    );
  }

  #[test]
  fn test_invalid_confidence_validation_error() {
    let gaming = NamedTempFile::new().unwrap();

    let config_content = format!(
      r#"
[ollama]
url = "http://localhost:11434"
model = "llama3.2:3b"

[[classifier]]
name = "gaming"
prompt_template = "{}"
min_confidence = 1.5
"#,
      gaming.path().display()
    );

    let config: Config = toml::from_str(&config_content).unwrap();
    let result = config.validate();
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(
      error_msg.contains("min_confidence must be between 0.0 and 1.0"),
      "Expected confidence error, got: {}",
      error_msg
    );
  }

  #[test]
  fn test_effective_values() {
    let gaming = NamedTempFile::new().unwrap();
    let video = NamedTempFile::new().unwrap();

    let config_content = format!(
      r#"
[ollama]
url = "http://localhost:11434"
model = "llama3.2:3b"

[defaults]
min_confidence = 0.8
ttl_days = 10

[[classifier]]
name = "gaming"
prompt_template = "{}"

[[classifier]]
name = "video-streaming"
prompt_template = "{}"
min_confidence = 0.9
ttl_days = 30
"#,
      gaming.path().display(),
      video.path().display()
    );

    let config: Config = toml::from_str(&config_content).unwrap();

    // First classifier uses defaults.
    assert_eq!(
      config.classifiers[0].effective_min_confidence(&config.defaults),
      0.8
    );
    assert_eq!(
      config.classifiers[0].effective_ttl_days(&config.defaults),
      10
    );

    // Second classifier overrides.
    assert_eq!(
      config.classifiers[1].effective_min_confidence(&config.defaults),
      0.9
    );
    assert_eq!(
      config.classifiers[1].effective_ttl_days(&config.defaults),
      30
    );
  }
}
