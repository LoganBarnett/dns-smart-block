mod db;

use axum::{
  extract::{Query, State},
  http::StatusCode,
  response::IntoResponse,
  routing::{get, post},
  Router,
};
use chrono::{DateTime, Utc};
use clap::Parser;
use dns_smart_block_common::logging::LoggingArgs;
use lazy_static::lazy_static;
use prometheus::register_int_counter;
use prometheus::register_int_counter_vec;
use prometheus::register_int_gauge;
use prometheus::register_int_gauge_vec;
use prometheus::{
  Encoder, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
  TextEncoder,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::path::PathBuf;
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

    static ref DOMAINS_CLASSIFIED_POSITIVE: IntGaugeVec = register_int_gauge_vec!(
        Opts::new("dns_smart_block_domains_classified_positive", "Currently valid positive classifications by type"),
        &["classification_type"]
    ).unwrap();

    static ref DOMAINS_CLASSIFIED_NEGATIVE: IntGaugeVec = register_int_gauge_vec!(
        Opts::new("dns_smart_block_domains_classified_negative", "Currently valid negative classifications by type"),
        &["classification_type"]
    ).unwrap();

    static ref DOMAINS_CLASSIFIED_TOTAL_CURRENT: IntGauge = register_int_gauge!(
        "dns_smart_block_domains_classified_total", "Total currently valid classified domains (all types)"
    ).unwrap();

    static ref DOMAINS_CLASSIFIED_POSITIVE_TOTAL: IntGauge = register_int_gauge!(
        "dns_smart_block_domains_classified_positive_total", "Total currently valid positive classifications"
    ).unwrap();

    static ref DOMAINS_CLASSIFIED_NEGATIVE_TOTAL: IntGauge = register_int_gauge!(
        "dns_smart_block_domains_classified_negative_total", "Total currently valid negative classifications"
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

    // Recent classification event metrics (time-windowed).
    static ref RECENT_CLASSIFICATIONS_5M: IntGaugeVec = register_int_gauge_vec!(
        Opts::new("dns_smart_block_recent_classifications_5m", "Classified events in the last 5 minutes by type"),
        &["classification_type"]
    ).unwrap();
}

#[derive(Parser, Debug)]
#[command(name = "dns-smart-block-blocklist-server")]
#[command(about = "Serves DNS blocklists from database classifications")]
struct CliArgs {
  #[command(flatten)]
  logging: LoggingArgs,

  /// PostgreSQL connection URL (without password if using password file)
  #[arg(long, env = "DATABASE_URL")]
  database_url: String,

  /// Path to file containing database password
  #[arg(long, env = "DATABASE_PASSWORD_FILE")]
  database_password_file: Option<PathBuf>,

  /// Address to bind the public server to (blocklist, metrics, health)
  #[arg(long, env = "PUBLIC_BIND_ADDRESS", default_value = "0.0.0.0:3000")]
  public_bind_address: String,

  /// Address to bind the admin server to (classifications, reprojection)
  #[arg(long, env = "ADMIN_BIND_ADDRESS", default_value = "127.0.0.1:8080")]
  admin_bind_address: String,
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
  match db::get_blocked_domains(
    &state.pool,
    &params.classification_type,
    check_time,
  )
  .await
  {
    Ok(domains) => {
      info!(
        "Serving {} domains for classification type '{}' at time {:?}",
        domains.len(),
        params.classification_type,
        check_time
          .map(|t| t.to_rfc3339())
          .unwrap_or_else(|| "now".to_string())
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
      for (classification_type, count) in &stats.current_classifications_by_type
      {
        DOMAINS_CLASSIFIED_CURRENT
          .with_label_values(&[classification_type])
          .set(*count);
      }

      // Update positive classification counts by type.
      for (classification_type, count) in &stats.current_positive_by_type {
        DOMAINS_CLASSIFIED_POSITIVE
          .with_label_values(&[classification_type])
          .set(*count);
      }

      // Update negative classification counts by type.
      for (classification_type, count) in &stats.current_negative_by_type {
        DOMAINS_CLASSIFIED_NEGATIVE
          .with_label_values(&[classification_type])
          .set(*count);
      }

      // Update total currently classified domains.
      DOMAINS_CLASSIFIED_TOTAL_CURRENT.set(stats.current_classifications_total);

      // Update total positive classifications.
      DOMAINS_CLASSIFIED_POSITIVE_TOTAL.set(stats.current_positive_total);

      // Update total negative classifications.
      DOMAINS_CLASSIFIED_NEGATIVE_TOTAL.set(stats.current_negative_total);

      // Update total unique domains seen.
      DOMAINS_SEEN_TOTAL.set(stats.domains_seen_total);

      // Update event counts by action.
      for (action, count) in &stats.events_by_action {
        EVENTS_BY_ACTION.with_label_values(&[action]).set(*count);
      }

      // Update cumulative classification counts by type.
      for (classification_type, count) in &stats.classifications_created_by_type
      {
        CLASSIFICATIONS_CREATED_TOTAL
          .with_label_values(&[classification_type])
          .set(*count);
      }

      // Update total cumulative classifications.
      CLASSIFICATIONS_CREATED_ALL_TOTAL
        .set(stats.classifications_created_total);

      // Update recent classification counts (last 5 minutes).
      for (classification_type, count) in &stats.recent_classified_by_type {
        RECENT_CLASSIFICATIONS_5M
          .with_label_values(&[classification_type])
          .set(*count);
      }
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
    Ok(_) => match String::from_utf8(buffer) {
      Ok(metrics_text) => (StatusCode::OK, metrics_text),
      Err(e) => {
        error!("Failed to convert metrics to UTF-8: {}", e);
        (
          StatusCode::INTERNAL_SERVER_ERROR,
          format!("Metrics encoding error: {}", e),
        )
      }
    },
    Err(e) => {
      error!("Failed to encode metrics: {}", e);
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Metrics encoding error: {}", e),
      )
    }
  }
}

#[derive(Deserialize)]
struct ClassificationsParams {
  classification_type: Option<String>,
}

async fn get_classifications(
  State(state): State<AppState>,
  Query(params): Query<ClassificationsParams>,
  headers: axum::http::HeaderMap,
) -> impl IntoResponse {
  match db::get_classifications(
    &state.pool,
    params.classification_type.as_deref(),
  )
  .await
  {
    Ok(classifications) => {
      info!(
        "Serving {} classifications{}",
        classifications.len(),
        params
          .classification_type
          .as_ref()
          .map(|ct| format!(" for type '{}'", ct))
          .unwrap_or_default()
      );

      // Check if client wants HTML.
      let wants_html = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false);

      if wants_html {
        let html = render_classifications_html(&classifications, &params);
        (
          StatusCode::OK,
          [(axum::http::header::CONTENT_TYPE, "text/html")],
          html,
        )
          .into_response()
      } else {
        (StatusCode::OK, axum::Json(classifications)).into_response()
      }
    }
    Err(e) => {
      error!("Database error fetching classifications: {}", e);
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(Vec::<db::ClassificationDetail>::new()),
      )
        .into_response()
    }
  }
}

fn render_classifications_html(
  classifications: &[db::ClassificationDetail],
  params: &ClassificationsParams,
) -> String {
  let filter_info = params
    .classification_type
    .as_ref()
    .map(|ct| format!(" (filtered by type: {})", ct))
    .unwrap_or_default();

  let rows: String = classifications
    .iter()
    .map(|c| {
      format!(
        r#"<tr>
          <td>{}</td>
          <td>{}</td>
          <td>{}</td>
          <td>{:.2}</td>
          <td class="reasoning">{}</td>
          <td>{}</td>
          <td>{}</td>
          <td>{}</td>
          <td>{}</td>
        </tr>"#,
        html_escape(&c.domain),
        html_escape(&c.classification_type),
        if c.is_matching_site { "Yes" } else { "No" },
        c.confidence,
        html_escape(c.reasoning.as_deref().unwrap_or("")),
        html_escape(&c.model),
        c.valid_on.format("%Y-%m-%d %H:%M:%S"),
        c.valid_until.format("%Y-%m-%d %H:%M:%S"),
        c.created_at.format("%Y-%m-%d %H:%M:%S"),
      )
    })
    .collect();

  format!(
    r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <title>Classifications{}</title>
  <style>
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      margin: 20px;
      background: #f5f5f5;
    }}
    h1 {{
      color: #333;
    }}
    table {{
      width: 100%;
      border-collapse: collapse;
      background: white;
      box-shadow: 0 2px 4px rgba(0,0,0,0.1);
    }}
    th, td {{
      padding: 12px;
      text-align: left;
      border-bottom: 1px solid #ddd;
    }}
    th {{
      background: #4CAF50;
      color: white;
      cursor: pointer;
      user-select: none;
      position: sticky;
      top: 0;
      z-index: 10;
    }}
    th:hover {{
      background: #45a049;
    }}
    th.sorted-asc::after {{
      content: " ▲";
    }}
    th.sorted-desc::after {{
      content: " ▼";
    }}
    tr:hover {{
      background: #f5f5f5;
    }}
    .reasoning {{
      max-width: 400px;
      white-space: normal;
      word-wrap: break-word;
    }}
    .count {{
      color: #666;
      font-size: 14px;
      margin-bottom: 10px;
    }}
  </style>
</head>
<body>
  <h1>Classifications{}</h1>
  <div class="count">Total: {} classification(s)</div>
  <table id="classificationsTable">
    <thead>
      <tr>
        <th onclick="sortTable(0)">Domain</th>
        <th onclick="sortTable(1)">Type</th>
        <th onclick="sortTable(2)">Match</th>
        <th onclick="sortTable(3)">Confidence</th>
        <th onclick="sortTable(4)">Reasoning</th>
        <th onclick="sortTable(5)">Model</th>
        <th onclick="sortTable(6)">Valid On</th>
        <th onclick="sortTable(7)">Valid Until</th>
        <th onclick="sortTable(8)">Created At</th>
      </tr>
    </thead>
    <tbody>
      {}
    </tbody>
  </table>
  <script>
    let sortDirection = {{}};

    function sortTable(columnIndex) {{
      const table = document.getElementById('classificationsTable');
      const tbody = table.querySelector('tbody');
      const rows = Array.from(tbody.querySelectorAll('tr'));

      const currentDirection = sortDirection[columnIndex] || 'asc';
      const newDirection = currentDirection === 'asc' ? 'desc' : 'asc';
      sortDirection = {{ [columnIndex]: newDirection }};

      rows.sort((a, b) => {{
        let aValue = a.cells[columnIndex].textContent.trim();
        let bValue = b.cells[columnIndex].textContent.trim();

        // Try to parse as date/time.
        const aDate = new Date(aValue);
        const bDate = new Date(bValue);

        if (!isNaN(aDate.getTime()) && !isNaN(bDate.getTime())) {{
          return newDirection === 'asc'
            ? aDate.getTime() - bDate.getTime()
            : bDate.getTime() - aDate.getTime();
        }}

        // Try to parse as number.
        const aNum = parseFloat(aValue);
        const bNum = parseFloat(bValue);

        if (!isNaN(aNum) && !isNaN(bNum)) {{
          return newDirection === 'asc' ? aNum - bNum : bNum - aNum;
        }}

        // String comparison.
        if (newDirection === 'asc') {{
          return aValue.localeCompare(bValue);
        }} else {{
          return bValue.localeCompare(aValue);
        }}
      }});

      rows.forEach(row => tbody.appendChild(row));

      // Update header indicators.
      table.querySelectorAll('th').forEach((th, idx) => {{
        th.classList.remove('sorted-asc', 'sorted-desc');
        if (idx === columnIndex) {{
          th.classList.add(`sorted-${{newDirection}}`);
        }}
      }});
    }}
  </script>
</body>
</html>"#,
    filter_info,
    filter_info,
    classifications.len(),
    rows
  )
}

fn html_escape(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&#x27;")
}

#[derive(Deserialize)]
struct ReprojectionParams {
  #[serde(default = "default_ttl_days")]
  ttl_days: i64,
}

fn default_ttl_days() -> i64 {
  10
}

async fn reprojection(
  State(state): State<AppState>,
  Query(params): Query<ReprojectionParams>,
) -> impl IntoResponse {
  info!("Starting reprojection with TTL {} days", params.ttl_days);

  match db::rebuild_projections_from_events(&state.pool, params.ttl_days).await
  {
    Ok(count) => {
      info!("Reprojection completed: {} classifications rebuilt", count);
      (
        StatusCode::OK,
        format!(
          "Reprojection completed: {} classifications rebuilt\n",
          count
        ),
      )
    }
    Err(e) => {
      error!("Reprojection failed: {}", e);
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Reprojection failed: {}\n", e),
      )
    }
  }
}

#[derive(Deserialize)]
struct ExpirationParams {
  domain: String,
}

async fn expire(
  State(state): State<AppState>,
  Query(params): Query<ExpirationParams>,
) -> impl IntoResponse {
  info!("Expiring domain '{}'", params.domain);
  let mut tx = state.pool.begin().await?;
  let expire_result = db::domain_expire(&mut tx, params.domain).await;
  tx.commit().await?;
  match expire_result {
    Ok(_) => {
      info!("Expired domain '{}'!", params.domain);
      (
        StatusCode::OK,
        format!(
          "Expired domain successful: {}", params.domain,
        )
      )
    },
    Err(e) => {
      error!("Domain expiration failed: {}", e);
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Domain expiration failed: {}\n", e),
      )
    }
  }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args = CliArgs::parse();

  // Initialize logging with auto-detection and CLI overrides
  args.logging.init_tracing();

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
    url
      .set_password(Some(&password))
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

  // Build public router (network accessible)
  let public_app = Router::new()
    .route("/blocklist", get(get_blocklist))
    .route("/health", get(health_check))
    .route("/metrics", get(metrics))
    .layer(TraceLayer::new_for_http())
    .with_state(state.clone());

  // Build admin router (localhost only)
  let admin_app = Router::new()
    .route("/classifications", get(get_classifications))
    .route("/reprojection", post(reprojection))
    .route("/expire", post(expire))
    .layer(TraceLayer::new_for_http())
    .with_state(state);

  // Parse bind addresses
  let public_addr: SocketAddr = args
    .public_bind_address
    .parse()
    .map_err(|e| format!("Invalid public bind address: {}", e))?;

  let admin_addr: SocketAddr = args
    .admin_bind_address
    .parse()
    .map_err(|e| format!("Invalid admin bind address: {}", e))?;

  info!("Public server listening on {}", public_addr);
  info!("Admin server listening on {}", admin_addr);

  // Spawn admin server in background
  let admin_listener = tokio::net::TcpListener::bind(admin_addr).await?;
  tokio::spawn(async move {
    if let Err(e) = axum::serve(admin_listener, admin_app).await {
      error!("Admin server error: {}", e);
    }
  });

  // Start public server (blocks)
  let public_listener = tokio::net::TcpListener::bind(public_addr).await?;
  axum::serve(public_listener, public_app).await?;

  Ok(())
}
