mod db;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use clap::Parser;
use lazy_static::lazy_static;
use prometheus::{Encoder, IntCounter, IntGauge, IntCounterVec, IntGaugeVec, Opts, Registry, TextEncoder};
use prometheus::register_int_counter;
use prometheus::register_int_gauge;
use prometheus::register_int_counter_vec;
use prometheus::register_int_gauge_vec;
use serde::Deserialize;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

lazy_static! {
    static ref REGISTRY: Registry = Registry::new();

    // Request tracking metrics.
    static ref BLOCKLIST_REQUESTS_TOTAL: IntCounterVec = register_int_counter_vec!(
        Opts::new("dns_smart_block_blocklist_requests_total", "Total number of blocklist requests"),
        &["classification_type", "status"]
    ).unwrap();

    static ref BLOCKLIST_DOMAINS_COUNT: IntGauge = register_int_gauge!(
        "dns_smart_block_blocklist_domains_total", "Total number of blocked domains across all classifications"
    ).unwrap();

    static ref HEALTH_CHECK_REQUESTS_TOTAL: IntCounter = register_int_counter!(
        "dns_smart_block_health_check_requests_total", "Total number of health check requests"
    ).unwrap();

    static ref METRICS_REQUESTS_TOTAL: IntCounter = register_int_counter!(
        "dns_smart_block_metrics_requests_total", "Total number of metrics requests"
    ).unwrap();

    // Database state metrics (gauges).
    static ref DOMAINS_CLASSIFIED_CURRENT: IntGaugeVec = register_int_gauge_vec!(
        Opts::new("dns_smart_block_domains_classified", "Currently valid classified domains by type"),
        &["classification_type"]
    ).unwrap();

    static ref DOMAINS_CLASSIFIED_TOTAL_CURRENT: IntGauge = register_int_gauge!(
        "dns_smart_block_domains_classified_total", "Total currently valid classified domains (all types)"
    ).unwrap();

    static ref DOMAINS_SEEN_TOTAL: IntGauge = register_int_gauge!(
        "dns_smart_block_domains_seen", "Total unique domains ever seen"
    ).unwrap();

    static ref EVENTS_BY_ACTION: IntGaugeVec = register_int_gauge_vec!(
        Opts::new("dns_smart_block_events", "Count of classification events by action"),
        &["action"]
    ).unwrap();

    // Cumulative metrics (counters represented as gauges for total counts).
    static ref CLASSIFICATIONS_CREATED_TOTAL: IntGaugeVec = register_int_gauge_vec!(
        Opts::new("dns_smart_block_classifications_total", "Total classifications ever created by type"),
        &["classification_type"]
    ).unwrap();

    static ref CLASSIFICATIONS_CREATED_ALL_TOTAL: IntGauge = register_int_gauge!(
        "dns_smart_block_classifications_all_total", "Total classifications ever created (all types)"
    ).unwrap();
}

#[derive(Parser, Debug)]
#[command(name = "dns-smart-block-blocklist-server")]
#[command(about = "Serves DNS blocklists from database classifications")]
struct CliArgs {
    /// PostgreSQL connection URL (without password if using password file)
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Path to file containing database password
    #[arg(long, env = "DATABASE_PASSWORD_FILE")]
    database_password_file: Option<PathBuf>,

    /// Address to bind the server to
    #[arg(long, env = "BIND_ADDRESS", default_value = "0.0.0.0:3000")]
    bind_address: String,
}

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

#[derive(Deserialize)]
struct BlocklistParams {
    /// Classification type (e.g., "gaming", "news", "sports")
    #[serde(rename = "type")]
    classification_type: String,

    /// Optional time to check (ISO 8601 format). Defaults to current time.
    #[serde(rename = "at", default)]
    at_time: Option<String>,
}

async fn get_blocklist(
    State(state): State<AppState>,
    Query(params): Query<BlocklistParams>,
) -> impl IntoResponse {
    // Parse the at_time parameter if provided.
    let check_time = if let Some(ref time_str) = params.at_time {
        match DateTime::parse_from_rfc3339(time_str) {
            Ok(dt) => Some(dt.with_timezone(&Utc)),
            Err(e) => {
                error!("Failed to parse time parameter '{}': {}", time_str, e);
                BLOCKLIST_REQUESTS_TOTAL
                    .with_label_values(&[params.classification_type.as_str(), "error"])
                    .inc();
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid time format. Use ISO 8601/RFC 3339 format: {}", e),
                );
            }
        }
    } else {
        None
    };

    // Query the database.
    match db::get_blocked_domains(&state.pool, &params.classification_type, check_time).await {
        Ok(domains) => {
            info!(
                "Serving {} domains for classification type '{}' at time {:?}",
                domains.len(),
                params.classification_type,
                check_time.map(|t| t.to_rfc3339()).unwrap_or_else(|| "now".to_string())
            );

            // Update metrics.
            BLOCKLIST_REQUESTS_TOTAL
                .with_label_values(&[params.classification_type.as_str(), "success"])
                .inc();
            BLOCKLIST_DOMAINS_COUNT.set(domains.len() as i64);

            // Return as plain text, one domain per line.
            let blocklist = domains.join("\n");
            (StatusCode::OK, blocklist)
        }
        Err(e) => {
            error!(
                "Database error while fetching blocklist for type '{}': {}",
                params.classification_type, e
            );
            BLOCKLIST_REQUESTS_TOTAL
                .with_label_values(&[params.classification_type.as_str(), "error"])
                .inc();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Internal server error: {}", e),
            )
        }
    }
}

async fn health_check() -> &'static str {
    HEALTH_CHECK_REQUESTS_TOTAL.inc();
    "OK"
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    METRICS_REQUESTS_TOTAL.inc();

    // Fetch database statistics and update gauge metrics.
    match db::get_metrics_stats(&state.pool).await {
        Ok(stats) => {
            // Update current classification counts by type.
            for (classification_type, count) in &stats.current_classifications_by_type {
                DOMAINS_CLASSIFIED_CURRENT
                    .with_label_values(&[classification_type])
                    .set(*count);
            }

            // Update total currently classified domains.
            DOMAINS_CLASSIFIED_TOTAL_CURRENT.set(stats.current_classifications_total);

            // Update total unique domains seen.
            DOMAINS_SEEN_TOTAL.set(stats.domains_seen_total);

            // Update event counts by action.
            for (action, count) in &stats.events_by_action {
                EVENTS_BY_ACTION
                    .with_label_values(&[action])
                    .set(*count);
            }

            // Update cumulative classification counts by type.
            for (classification_type, count) in &stats.classifications_created_by_type {
                CLASSIFICATIONS_CREATED_TOTAL
                    .with_label_values(&[classification_type])
                    .set(*count);
            }

            // Update total cumulative classifications.
            CLASSIFICATIONS_CREATED_ALL_TOTAL.set(stats.classifications_created_total);
        }
        Err(e) => {
            error!("Failed to fetch database metrics: {}", e);
            // Continue serving metrics even if DB query fails.
        }
    }

    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();

    match encoder.encode(&metric_families, &mut buffer) {
        Ok(_) => {
            match String::from_utf8(buffer) {
                Ok(metrics_text) => (StatusCode::OK, metrics_text),
                Err(e) => {
                    error!("Failed to convert metrics to UTF-8: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, format!("Metrics encoding error: {}", e))
                }
            }
        }
        Err(e) => {
            error!("Failed to encode metrics: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Metrics encoding error: {}", e))
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = CliArgs::parse();

    info!("Starting DNS Smart Block Blocklist Server");

    // Construct database URL with password if provided
    let database_url = if let Some(password_file) = args.database_password_file {
        let password = std::fs::read_to_string(&password_file)
            .map_err(|e| {
                error!("Failed to read database password file: {}", e);
                e
            })?
            .trim()
            .to_string();

        let mut url = url::Url::parse(&args.database_url)?;
        url.set_password(Some(&password))
            .map_err(|_| "Failed to set password in URL")?;
        url.to_string()
    } else {
        args.database_url.clone()
    };

    // Sanitize URL for logging
    let sanitized_url = if let Ok(mut url) = url::Url::parse(&database_url) {
        if url.password().is_some() {
            let _ = url.set_password(Some("***"));
        }
        url.to_string()
    } else {
        database_url.clone()
    };

    info!("Database URL: {}", sanitized_url);

    // Connect to PostgreSQL
    info!("Connecting to PostgreSQL...");
    let pool = PgPool::connect(&database_url).await?;
    info!("Connected to PostgreSQL successfully");

    // Build app state
    let state = AppState { pool };

    // Build router.
    let app = Router::new()
        .route("/blocklist", get(get_blocklist))
        .route("/health", get(health_check))
        .route("/metrics", get(metrics))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Parse bind address
    let addr: SocketAddr = args
        .bind_address
        .parse()
        .map_err(|e| format!("Invalid bind address: {}", e))?;

    info!("Listening on {}", addr);

    // Start server
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
