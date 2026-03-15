mod database_url;
mod db;

use clap::Parser;
use database_url::{construct_database_url, sanitize_database_url};
use db::DbError;
use dns_smart_block_classifier::{compute_prompt_hash, output::ClassificationOutput};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::path::PathBuf;
use std::process::Stdio;
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "dns-smart-block-queue-processor")]
#[command(about = "Processes domains from NATS queue and classifies them")]
struct CliArgs {
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

    /// Ollama server URL
    #[arg(long, env = "OLLAMA_URL", default_value = "http://localhost:11434")]
    ollama_url: String,

    /// Ollama model to use
    #[arg(long, env = "OLLAMA_MODEL", default_value = "llama2")]
    ollama_model: String,

    /// Path to prompt template file
    #[arg(long, env = "PROMPT_TEMPLATE")]
    prompt_template: PathBuf,

    /// Classification type (e.g., "gaming")
    #[arg(long, env = "CLASSIFICATION_TYPE", default_value = "gaming")]
    classification_type: String,

    /// HTTP timeout in seconds for fetching domains
    #[arg(long, env = "HTTP_TIMEOUT_SEC", default_value = "10")]
    http_timeout_sec: u64,

    /// Maximum KB to download from each domain
    #[arg(long, env = "HTTP_MAX_KB", default_value = "100")]
    http_max_kb: usize,

    /// Minimum confidence threshold to block (0.0 to 1.0)
    #[arg(long, env = "MIN_CONFIDENCE", default_value = "0.8")]
    min_confidence: f64,

    /// TTL for classifications in days
    #[arg(long, env = "CLASSIFICATION_TTL_DAYS", default_value = "10")]
    ttl_days: i64,
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
}

type Result<T> = std::result::Result<T, ProcessorError>;

async fn run_classifier(
    domain: &str,
    args: &CliArgs,
) -> Result<ClassificationOutput> {
    info!("Running classifier for domain: {}", domain);

    let mut child = Command::new(&args.classifier_path)
        .arg("--domain")
        .arg(domain)
        .arg("--ollama-url")
        .arg(&args.ollama_url)
        .arg("--ollama-model")
        .arg(&args.ollama_model)
        .arg("--prompt-template")
        .arg(&args.prompt_template)
        .arg("--classification-type")
        .arg(&args.classification_type)
        .arg("--http-timeout-sec")
        .arg(args.http_timeout_sec.to_string())
        .arg("--http-max-kb")
        .arg(args.http_max_kb.to_string())
        .arg("--output")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Read stdout and stderr concurrently (actually concurrent this time)
    let (stdout_result, stderr_result) = tokio::join!(
        async {
            let mut buf = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                stdout.read_to_string(&mut buf).await?;
            }
            Ok::<String, std::io::Error>(buf)
        },
        async {
            let mut buf = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                stderr.read_to_string(&mut buf).await?;
            }
            Ok::<String, std::io::Error>(buf)
        }
    );

    let stdout_buf = stdout_result?;
    let stderr_buf = stderr_result?;

    let _status = child.wait().await?;

    // Log stderr (classifier logs)
    if !stderr_buf.is_empty() {
        info!("Classifier stderr:\n{}", stderr_buf);
    }

    // Parse stdout as JSON
    if stdout_buf.is_empty() {
        return Err(ProcessorError::ClassifierError(
            "Classifier produced no output".to_string(),
        ));
    }

    info!("Classifier stdout: {}", stdout_buf);

    // Try to parse as ClassificationOutput
    match serde_json::from_str::<ClassificationOutput>(&stdout_buf) {
        Ok(output) => {
            if output.result == "classified" {
                Ok(output)
            } else {
                Err(ProcessorError::ClassifierError(format!(
                    "Classifier returned non-classified result: {}",
                    output.result
                )))
            }
        }
        Err(_) => {
            // Try to parse as ErrorOutput
            match serde_json::from_str::<
                dns_smart_block_classifier::output::ErrorOutput,
            >(&stdout_buf)
            {
                Ok(error_output) => Err(ProcessorError::ClassifierError(
                    format!(
                        "{}: {}",
                        error_output.error.error_type, error_output.error.message
                    ),
                )),
                Err(e) => Err(ProcessorError::ClassifierError(format!(
                    "Failed to parse classifier output: {}. Output was: {}",
                    e, stdout_buf
                ))),
            }
        }
    }
}

async fn process_domain(
    domain: &str,
    args: &CliArgs,
    pool: &PgPool,
    prompt_template: &str,
) -> Result<()> {
    info!("Processing domain: {}", domain);

    // Insert "classifying" event
    db::insert_event(
        pool,
        domain,
        "classifying",
        json!({
            "model": args.ollama_model,
            "prompt_hash": compute_prompt_hash(prompt_template)
        }),
    )
    .await?;

    // Run classifier
    match run_classifier(domain, args).await {
        Ok(output) => {
            info!(
                "Classification successful for {}: is_matching={}, confidence={}",
                domain,
                output.classification.is_matching_site,
                output.classification.confidence
            );

            // Insert "classified" event
            db::insert_event(
                pool,
                domain,
                "classified",
                json!({
                    "is_matching_site": output.classification.is_matching_site,
                    "confidence": output.classification.confidence,
                    "classification_type": args.classification_type,
                    "http_status": output.metadata.http_status,
                }),
            )
            .await?;

            // Update projections if it's a matching site above threshold
            if output.classification.is_matching_site
                && output.classification.confidence >= args.min_confidence
            {
                info!(
                    "Domain {} matches criteria, updating projections",
                    domain
                );

                db::update_projections(
                    pool,
                    domain,
                    &args.classification_type,
                    output.classification.confidence,
                    &args.ollama_model,
                    prompt_template,
                    &output.metadata.prompt_hash,
                    args.ttl_days,
                )
                .await?;

                info!("Projections updated successfully for {}", domain);
            } else {
                info!(
                    "Domain {} does not match criteria or below confidence threshold",
                    domain
                );
            }

            Ok(())
        }
        Err(e) => {
            error!("Classification failed for {}: {}", domain, e);

            // Insert "error" event
            db::insert_event(
                pool,
                domain,
                "error",
                json!({
                    "error": e.to_string(),
                }),
            )
            .await?;

            // Determine if this is a permanent or transient error
            let is_permanent_error = match &e {
                ProcessorError::ClassifierError(msg) => {
                    // Check if it's a DNS resolution error, invalid domain, or
                    // other permanent failure
                    msg.contains("dns_resolution_failed")
                        || msg.contains("invalid_domain")
                        || msg.contains("http_fetch_failed: 404")
                        || msg.contains("http_fetch_failed: 403")
                }
                // Database and I/O errors are generally transient
                ProcessorError::DatabaseError(_)
                | ProcessorError::IoError(_)
                | ProcessorError::SqlxError(_) => false,
                // Other errors are considered transient
                _ => false,
            };

            if is_permanent_error {
                info!(
                    "Permanent error for {}, will not retry: {}",
                    domain, e
                );
                return Ok(());
            }

            // Check consecutive failures
            let consecutive_errors = db::count_consecutive_errors(pool, domain, 10).await?;

            if consecutive_errors >= 3 {
                warn!(
                    "Domain {} has {} consecutive failures, will not retry",
                    domain, consecutive_errors
                );
                // Don't retry - too many failures
                Ok(())
            } else {
                info!(
                    "Domain {} has {} consecutive failures, will retry",
                    domain, consecutive_errors
                );
                // Propagate error to trigger NAK and retry
                Err(e)
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = CliArgs::parse();

    info!("Starting DNS Smart Block Queue Processor");
    info!("NATS URL: {}", args.nats_url);
    info!("NATS subject: {}", args.nats_subject);
    info!("Classifier path: {}", args.classifier_path);
    info!("Ollama URL: {}", args.ollama_url);
    info!("Ollama model: {}", args.ollama_model);

    // Construct database URL with password if provided
    let database_url = construct_database_url(
        &args.database_url,
        args.database_password_file.as_deref(),
    )?;

    info!("Database URL: {}", sanitize_database_url(&database_url));

    // Load prompt template
    let prompt_template = std::fs::read_to_string(&args.prompt_template)
        .map_err(|e| {
            error!(
                "Failed to read prompt template from {:?}: {}",
                args.prompt_template, e
            );
            e
        })?;

    info!("Loaded prompt template from {:?}", args.prompt_template);
    info!("Prompt hash: {}", compute_prompt_hash(&prompt_template));

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

    // Create or get a durable consumer for this processor type
    // Each processor type (gaming, video-streaming) gets its own consumer
    let consumer_name = format!("dns-smart-block-{}", args.classification_type);
    info!("Creating JetStream consumer: {}", consumer_name);

    let stream = jetstream
        .get_stream("DNS_SMART_BLOCK")
        .await
        .map_err(|e| ProcessorError::NatsError(format!("Failed to get stream: {}", e)))?;

    let consumer = stream
        .get_or_create_consumer(
            &consumer_name,
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some(consumer_name.clone()),
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

                // Process the domain
                match process_domain(
                    &domain_msg.domain,
                    &args,
                    &pool,
                    &prompt_template,
                )
                .await
                {
                    Ok(_) => {
                        // Acknowledge the message after successful processing
                        if let Err(e) = message.ack().await {
                            error!("Failed to acknowledge message: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Error processing domain {}: {}", domain_msg.domain, e);
                        // Don't acknowledge failed messages - they'll be redelivered
                        if let Err(nak_err) = message.ack_with(async_nats::jetstream::AckKind::Nak(None)).await {
                            error!("Failed to NAK message: {}", nak_err);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to deserialize message: {}", e);
                warn!("Raw payload: {:?}", String::from_utf8_lossy(&payload));
                // Acknowledge malformed messages so they don't get redelivered
                if let Err(ack_err) = message.ack().await {
                    error!("Failed to acknowledge malformed message: {}", ack_err);
                }
            }
        }
    }

    info!("NATS subscription ended");
    Ok(())
}
