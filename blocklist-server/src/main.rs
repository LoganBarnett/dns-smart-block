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
use serde::Deserialize;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

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
    // Parse the at_time parameter if provided
    let check_time = if let Some(ref time_str) = params.at_time {
        match DateTime::parse_from_rfc3339(time_str) {
            Ok(dt) => Some(dt.with_timezone(&Utc)),
            Err(e) => {
                error!("Failed to parse time parameter '{}': {}", time_str, e);
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid time format. Use ISO 8601/RFC 3339 format: {}", e),
                );
            }
        }
    } else {
        None
    };

    // Query the database
    match db::get_blocked_domains(&state.pool, &params.classification_type, check_time).await {
        Ok(domains) => {
            info!(
                "Serving {} domains for classification type '{}' at time {:?}",
                domains.len(),
                params.classification_type,
                check_time.map(|t| t.to_rfc3339()).unwrap_or_else(|| "now".to_string())
            );

            // Return as plain text, one domain per line
            let blocklist = domains.join("\n");
            (StatusCode::OK, blocklist)
        }
        Err(e) => {
            error!(
                "Database error while fetching blocklist for type '{}': {}",
                params.classification_type, e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Internal server error: {}", e),
            )
        }
    }
}

async fn health_check() -> &'static str {
    "OK"
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

    // Build router
    let app = Router::new()
        .route("/blocklist", get(get_blocklist))
        .route("/health", get(health_check))
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
