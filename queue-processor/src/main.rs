mod config;
mod database_url;
mod db;

use clap::Parser;
use config::{ClassifierConfig, Config};
use database_url::{construct_database_url, sanitize_database_url};
use db::{ClassifierState, DbError};
use dns_smart_block_classifier::{compute_prompt_hash, output::ClassificationOutput};
use dns_smart_block_common::logging::LoggingArgs;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::path::PathBuf;
use std::process::Stdio;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "dns-smart-block-queue-processor")]
#[command(about = "Processes domains from NATS queue and classifies them")]
struct CliArgs {
    #[command(flatten)]
    logging: LoggingArgs,

    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,

    /// NATS subject to subscribe to
    #[arg(long, env = "NATS_SUBJECT", default_value = "dns.domains")]
    nats_subject: String,

    /// Maximum number of unacknowledged messages per consumer
    #[arg(long, env = "NATS_MAX_ACK_PENDING", default_value = "1")]
    nats_max_ack_pending: i64,

    /// PostgreSQL connection URL (without password if using password file)
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Path to file containing database password
    #[arg(long, env = "DATABASE_PASSWORD_FILE")]
    database_password_file: Option<PathBuf>,

    /// Path to classifier binary
    #[arg(
        long,
        env = "CLASSIFIER_PATH",
        default_value = "dns-smart-block-classifier"
    )]
    classifier_path: String,

    /// Path to TOML configuration file
    #[arg(long, env = "CONFIG_FILE")]
    config_file: PathBuf,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DomainMessage {
    domain: String,
    timestamp: i64,
}

#[derive(Error, Debug)]
enum ProcessorError {
    #[error("NATS error: {0}")]
    NatsError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] DbError),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Classifier execution error: {0}")]
    ClassifierError(String),

    #[error("SQL error: {0}")]
    SqlxError(#[from] sqlx::Error),

    #[error("Migration error: {0}")]
    MigrateError(#[from] sqlx::migrate::MigrateError),

    #[error("Database URL error: {0}")]
    DatabaseUrlError(#[from] database_url::DatabaseUrlError),

    #[error("Configuration error: {0}")]
    ConfigError(#[from] config::ConfigError),
}

type Result<T> = std::result::Result<T, ProcessorError>;

async fn run_classifier(
    domain: &str,
    classifier_config: &ClassifierConfig,
    config: &Config,
    classifier_path: &str,
) -> Result<ClassificationOutput> {
    info!(
        "Running classifier '{}' for domain: {}",
        classifier_config.name, domain
    );

    let ollama_model = classifier_config.effective_ollama_model(&config.ollama);
    let http_timeout_sec = classifier_config.effective_http_timeout_sec(&config.http);
    let http_max_kb = classifier_config.effective_http_max_kb(&config.http);

    let mut child = Command::new(classifier_path)
        .arg("--domain")
        .arg(domain)
        .arg("--ollama-url")
        .arg(&config.ollama.url)
        .arg("--ollama-model")
        .arg(&ollama_model)
        .arg("--prompt-template")
        .arg(&classifier_config.prompt_template)
        .arg("--classification-type")
        .arg(&classifier_config.name)
        .arg("--http-timeout-sec")
        .arg(http_timeout_sec.to_string())
        .arg("--http-max-kb")
        .arg(http_max_kb.to_string())
        .arg("--output")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Read stdout and stderr concurrently.
    // stdout: buffered until completion (small JSON payload)
    // stderr: streamed line-by-line for live logging
    let (stdout_result, stderr_result) = tokio::join!(
        async {
            let mut buf = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                stdout.read_to_string(&mut buf).await?;
            }
            Ok::<String, std::io::Error>(buf)
        },
        async {
            if let Some(stderr) = child.stderr.take() {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();

                while let Some(line) = lines.next_line().await? {
                    info!(
                        classifier = %classifier_config.name,
                        "{}",
                        line
                    );
                }
            }
            Ok::<(), std::io::Error>(())
        }
    );

    let stdout_buf = stdout_result?;
    stderr_result?;

    let _status = child.wait().await?;

    // Parse stdout as JSON
    if stdout_buf.is_empty() {
        return Err(ProcessorError::ClassifierError(format!(
            "Classifier '{}' produced no output",
            classifier_config.name
        )));
    }

    info!(
        "Classifier '{}' stdout: {}",
        classifier_config.name, stdout_buf
    );

    // Try to parse as ClassificationOutput
    match serde_json::from_str::<ClassificationOutput>(&stdout_buf) {
        Ok(output) => {
            if output.result == "classified" {
                Ok(output)
            } else {
                Err(ProcessorError::ClassifierError(format!(
                    "Classifier '{}' returned non-classified result: {}",
                    classifier_config.name, output.result
                )))
            }
        }
        Err(_) => {
            // Try to parse as ErrorOutput
            match serde_json::from_str::<
                dns_smart_block_classifier::output::ErrorOutput,
            >(&stdout_buf)
            {
                Ok(error_output) => Err(ProcessorError::ClassifierError(format!(
                    "Classifier '{}': {}: {}",
                    classifier_config.name,
                    error_output.error.error_type,
                    error_output.error.message
                ))),
                Err(e) => Err(ProcessorError::ClassifierError(format!(
                    "Classifier '{}': Failed to parse output: {}. Output was: {}",
                    classifier_config.name, e, stdout_buf
                ))),
            }
        }
    }
}

async fn process_domain(
    domain: &str,
    config: &Config,
    pool: &PgPool,
    classifier_path: &str,
) -> Result<()> {
    info!("Processing domain: {}", domain);

    // Get all classifier states in a single query.
    let classification_types: Vec<String> = config
        .classifiers
        .iter()
        .map(|c| c.name.clone())
        .collect();

    let states = db::get_classifier_states(pool, domain, &classification_types).await?;

    // Process each classifier based on its state.
    for (classification_type, state) in states {
        let classifier_config = config
            .classifiers
            .iter()
            .find(|c| c.name == classification_type)
            .expect("Classifier config should exist for all classification types");

        match state {
            ClassifierState::Current => {
                info!(
                    "Skipping classifier '{}' for domain {}: classification is current",
                    classification_type, domain
                );
                continue;
            }
            ClassifierState::Expired => {
                info!(
                    "Running classifier '{}' for domain {}: classification expired",
                    classification_type, domain
                );
            }
            ClassifierState::Error => {
                info!(
                    "Running classifier '{}' for domain {}: previous attempt failed",
                    classification_type, domain
                );
            }
            ClassifierState::Missing => {
                info!(
                    "Running classifier '{}' for domain {}: no classification exists",
                    classification_type, domain
                );
            }
        }

        // Load prompt template for this classifier.
        let prompt_template = match std::fs::read_to_string(&classifier_config.prompt_template) {
            Ok(content) => content,
            Err(e) => {
                error!(
                    "Failed to read prompt template for classifier '{}' from {:?}: {}",
                    classifier_config.name, classifier_config.prompt_template, e
                );

                // Insert error event and continue to next classifier.
                db::insert_event(
                    pool,
                    domain,
                    "error",
                    json!({
                        "classification_type": classifier_config.name,
                        "error": format!("Failed to read prompt template: {}", e),
                    }),
                )
                .await?;
                continue;
            }
        };

        // Insert "classifying" event.
        db::insert_event(
            pool,
            domain,
            "classifying",
            json!({
                "classification_type": classifier_config.name,
                "model": classifier_config.effective_ollama_model(&config.ollama),
                "prompt_hash": compute_prompt_hash(&prompt_template),
            }),
        )
        .await?;

        // Run the classifier.
        match run_classifier(domain, classifier_config, config, classifier_path).await
        {
            Ok(output) => {
                info!(
                    "Classifier '{}' successful for {}: is_matching={}, confidence={}",
                    classifier_config.name,
                    domain,
                    output.classification.is_matching_site,
                    output.classification.confidence
                );

                // Insert "classified" event.
                db::insert_event(
                    pool,
                    domain,
                    "classified",
                    json!({
                        "classification_type": classifier_config.name,
                        "is_matching_site": output.classification.is_matching_site,
                        "confidence": output.classification.confidence,
                        "http_status": output.metadata.http_status,
                    }),
                )
                .await?;

                // Update projections if it's a matching site above threshold.
                let min_confidence = classifier_config.effective_min_confidence(&config.defaults);
                let ttl_days = classifier_config.effective_ttl_days(&config.defaults);

                if output.classification.is_matching_site
                    && output.classification.confidence >= min_confidence
                {
                    info!(
                        "Domain {} matches criteria for classifier '{}', updating projections",
                        domain, classifier_config.name
                    );

                    db::update_projections(
                        pool,
                        domain,
                        &classifier_config.name,
                        output.classification.confidence,
                        &output.classification.reasoning,
                        &classifier_config.effective_ollama_model(&config.ollama),
                        &prompt_template,
                        &output.metadata.prompt_hash,
                        ttl_days,
                    )
                    .await?;

                    info!(
                        "Projections updated successfully for {} (classifier '{}')",
                        domain, classifier_config.name
                    );
                } else {
                    info!(
                        "Domain {} does not match criteria or below confidence threshold for classifier '{}'",
                        domain, classifier_config.name
                    );
                }
            }
            Err(e) => {
                error!(
                    "Classifier '{}' failed for {}: {}",
                    classifier_config.name, domain, e
                );

                // Insert "error" event.
                db::insert_event(
                    pool,
                    domain,
                    "error",
                    json!({
                        "classification_type": classifier_config.name,
                        "error": e.to_string(),
                    }),
                )
                .await?;

                // Continue to next classifier - we don't fail the whole domain
                // processing just because one classifier failed. The error is
                // recorded and will be retried on the next DNS query.
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();

    // Initialize logging with auto-detection and CLI overrides
    args.logging.init_tracing();

    info!("Starting DNS Smart Block Queue Processor");
    info!("NATS URL: {}", args.nats_url);
    info!("NATS subject: {}", args.nats_subject);
    info!("Classifier path: {}", args.classifier_path);
    info!("Config file: {}", args.config_file.display());

    // Load configuration file
    info!("Loading configuration from {:?}", args.config_file);
    let config = Config::from_file(&args.config_file)?;

    info!("Configuration loaded successfully");
    info!("Ollama URL: {}", config.ollama.url);
    info!("Ollama model (default): {}", config.ollama.model);
    info!("Number of classifiers: {}", config.classifiers.len());
    for classifier in &config.classifiers {
        info!(
            "  - {} (prompt: {:?})",
            classifier.name, classifier.prompt_template
        );
    }

    // Construct database URL with password if provided
    let database_url = construct_database_url(
        &args.database_url,
        args.database_password_file.as_deref(),
    )?;

    info!("Database URL: {}", sanitize_database_url(&database_url));

    // Connect to PostgreSQL
    info!("Connecting to PostgreSQL...");
    let pool = PgPool::connect(&database_url).await?;
    info!("Connected to PostgreSQL successfully");

    // Run migrations
    info!("Running database migrations...");
    sqlx::migrate!("../migrations").run(&pool).await?;
    info!("Migrations completed successfully");

    // Connect to NATS
    info!("Connecting to NATS...");
    let client = async_nats::connect(&args.nats_url)
        .await
        .map_err(|e| ProcessorError::NatsError(e.to_string()))?;

    info!("Connected to NATS successfully");

    // Get JetStream context
    let jetstream = async_nats::jetstream::new(client);

    // Create or get a single durable consumer for all classifiers
    let consumer_name = "dns-smart-block-queue-processor";
    info!("Creating JetStream consumer: {}", consumer_name);

    let stream = jetstream
        .get_stream("DNS_SMART_BLOCK")
        .await
        .map_err(|e| ProcessorError::NatsError(format!("Failed to get stream: {}", e)))?;

    let consumer = stream
        .get_or_create_consumer(
            consumer_name,
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some(consumer_name.to_string()),
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                max_ack_pending: args.nats_max_ack_pending,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| ProcessorError::NatsError(format!("Failed to create consumer: {}", e)))?;

    info!("JetStream consumer created, waiting for messages...");

    // Process messages from JetStream
    let mut messages = consumer
        .messages()
        .await
        .map_err(|e| ProcessorError::NatsError(format!("Failed to get message stream: {}", e)))?;

    while let Some(message) = messages.next().await {
        let message = match message {
            Ok(msg) => msg,
            Err(e) => {
                error!("Error receiving message: {}", e);
                continue;
            }
        };

        let payload = message.payload.clone();

        // Deserialize domain message
        match serde_json::from_slice::<DomainMessage>(&payload) {
            Ok(domain_msg) => {
                info!(
                    "Received domain: {} (timestamp: {})",
                    domain_msg.domain, domain_msg.timestamp
                );

                // Process the domain (runs all needed classifiers).
                // We always ACK the message regardless of success or failure.
                // Errors are recorded in the database and will be retried on
                // the next DNS query for this domain.
                match process_domain(&domain_msg.domain, &config, &pool, &args.classifier_path)
                    .await
                {
                    Ok(_) => {
                        info!("Successfully processed domain: {}", domain_msg.domain);
                    }
                    Err(e) => {
                        error!("Error processing domain {}: {}", domain_msg.domain, e);
                    }
                }

                // Always acknowledge the message.
                if let Err(e) = message.ack().await {
                    error!("Failed to acknowledge message: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to deserialize message: {}", e);
                warn!("Raw payload: {:?}", String::from_utf8_lossy(&payload));
                // Acknowledge malformed messages so they don't get redelivered.
                if let Err(ack_err) = message.ack().await {
                    error!("Failed to acknowledge malformed message: {}", ack_err);
                }
            }
        }
    }

    info!("NATS subscription ended");
    Ok(())
}
