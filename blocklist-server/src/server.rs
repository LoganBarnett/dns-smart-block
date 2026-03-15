use crate::cli::CliArgs;
use crate::db;
use crate::metrics;
use axum::http::header;
use axum::{
  Router,
  extract::{Query, State},
  http::StatusCode,
  response::IntoResponse,
  routing::{get, post},
};
use chrono::{DateTime, Utc};
use dns_smart_block_blocklist_server::{
  CLASSIFICATIONS_CSS, CLASSIFICATIONS_HTML, CLASSIFICATIONS_JS,
};
use dns_smart_block_common::db::{
  DomainExpire, DomainRequeue, ErroredClassification,
};
use prometheus::{Encoder, TextEncoder};
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashSet;
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct NatsState {
  client: async_nats::Client,
  subject: String,
}

#[derive(Clone)]
pub struct AppState {
  pool: PgPool,
  nats: Option<NatsState>,
}

async fn publish_to_nats(nats: &NatsState, domain: &str) -> Result<(), String> {
  let payload = serde_json::json!({
    "domain": domain,
    "timestamp": Utc::now().timestamp()
  });
  let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
  nats
    .client
    .publish(nats.subject.clone(), bytes.into())
    .await
    .map_err(|e| e.to_string())?;
  Ok(())
}

// ── public handlers ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BlocklistParams {
  /// Classification type (e.g., "gaming", "news", "sports")
  #[serde(rename = "type")]
  classification_type: String,

  /// Optional time to check (ISO 8601 format).  Defaults to current time.
  #[serde(rename = "at", default)]
  at_time: Option<String>,
}

async fn get_blocklist(
  State(state): State<AppState>,
  Query(params): Query<BlocklistParams>,
) -> impl IntoResponse {
  let check_time = if let Some(ref time_str) = params.at_time {
    match DateTime::parse_from_rfc3339(time_str) {
      Ok(dt) => Some(dt.with_timezone(&Utc)),
      Err(e) => {
        error!("Failed to parse time parameter '{}': {}", time_str, e);
        metrics::BLOCKLIST_REQUESTS_TOTAL
          .with_label_values(&[params.classification_type.as_str(), "error"])
          .inc();
        return (
          StatusCode::BAD_REQUEST,
          format!("Invalid time format.  Use ISO 8601/RFC 3339 format: {}", e),
        );
      }
    }
  } else {
    None
  };

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

      metrics::BLOCKLIST_REQUESTS_TOTAL
        .with_label_values(&[params.classification_type.as_str(), "success"])
        .inc();
      metrics::BLOCKLIST_LAST_REQUEST_TIMESTAMP
        .with_label_values(&[params.classification_type.as_str()])
        .set(Utc::now().timestamp() as f64);
      metrics::BLOCKLIST_DOMAINS_COUNT.set(domains.len() as i64);

      let blocklist = domains.join("\n");
      (StatusCode::OK, blocklist)
    }
    Err(e) => {
      error!(
        "Database error while fetching blocklist for type '{}': {}",
        params.classification_type, e
      );
      metrics::BLOCKLIST_REQUESTS_TOTAL
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
  metrics::HEALTH_CHECK_REQUESTS_TOTAL.inc();
  "OK"
}

async fn prometheus_metrics(
  State(state): State<AppState>,
) -> impl IntoResponse {
  metrics::METRICS_REQUESTS_TOTAL.inc();

  match db::get_metrics_stats(&state.pool).await {
    Ok(stats) => metrics::update_from_stats(&stats),
    Err(e) => error!("Failed to fetch database metrics: {}", e),
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

// ── admin handlers ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ClassificationsParams {
  classification_type: Option<String>,
}

async fn get_classifications(
  State(state): State<AppState>,
  Query(params): Query<ClassificationsParams>,
  headers: axum::http::HeaderMap,
) -> impl IntoResponse {
  let classifications = match db::get_classifications(
    &state.pool,
    params.classification_type.as_deref(),
  )
  .await
  {
    Ok(c) => c,
    Err(e) => {
      error!("Database error fetching classifications: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(Vec::<db::ClassificationDetail>::new()),
      )
        .into_response();
    }
  };

  info!(
    "Serving {} classifications{}",
    classifications.len(),
    params
      .classification_type
      .as_ref()
      .map(|ct| format!(" for type '{}'", ct))
      .unwrap_or_default()
  );

  let wants_html = headers
    .get(axum::http::header::ACCEPT)
    .and_then(|v| v.to_str().ok())
    .map(|v| v.contains("text/html"))
    .unwrap_or(false);

  if wants_html {
    let errored = ErroredClassification::find(
      &state.pool,
      params.classification_type.as_deref(),
    )
    .await
    .unwrap_or_else(|e| {
      error!("Database error fetching errored classifications: {}", e);
      vec![]
    });

    let html = render_classifications_html(&classifications, &errored, &params);
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

fn render_classifications_html(
  classifications: &[db::ClassificationDetail],
  errored: &[ErroredClassification],
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
          <td><button class="expire-btn" onclick="expireDomain('{}')">Expire</button></td>
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
        html_escape(&c.domain),
      )
    })
    .collect();

  let requeue_type_buttons: String = {
    let mut seen = HashSet::new();
    errored
      .iter()
      .filter(|e| seen.insert(e.classification_type.clone()))
      .map(|e| {
        let type_count = errored
          .iter()
          .filter(|x| x.classification_type == e.classification_type)
          .count();
        format!(
          r#"<button class="requeue-btn" onclick="requeueType('{}')">Requeue {} errors ({})</button>"#,
          html_escape(&e.classification_type),
          html_escape(&e.classification_type),
          type_count,
        )
      })
      .collect::<Vec<_>>()
      .join("\n")
  };

  let admin_actions = if errored.is_empty() {
    String::new()
  } else {
    format!(
      r#"<div class="admin-actions">
  <button class="requeue-btn requeue-all-btn" onclick="requeueAll()">Requeue all errors ({})</button>
  {}
</div>"#,
      errored.len(),
      requeue_type_buttons,
    )
  };

  let errors_section = if errored.is_empty() {
    String::new()
  } else {
    let error_rows: String = errored
      .iter()
      .map(|e| {
        format!(
          r#"<tr class="error-row">
          <td>{}</td>
          <td>{}</td>
          <td class="reasoning">{}</td>
          <td>{}</td>
          <td><button class="requeue-btn" onclick="requeueDomain('{}', '{}')">Requeue</button></td>
        </tr>"#,
          html_escape(&e.domain),
          html_escape(&e.classification_type),
          html_escape(e.error_message.as_deref().unwrap_or("")),
          e.errored_at.format("%Y-%m-%d %H:%M:%S"),
          html_escape(&e.domain),
          html_escape(&e.classification_type),
        )
      })
      .collect();

    format!(
      r#"<h2>Errored Classifications</h2>
<div class="count">Total: {} error(s)</div>
<table id="errorsTable">
  <thead>
    <tr>
      <th onclick="sortTable('errorsTable', 0)">Domain</th>
      <th onclick="sortTable('errorsTable', 1)">Type</th>
      <th onclick="sortTable('errorsTable', 2)">Error</th>
      <th onclick="sortTable('errorsTable', 3)">Errored At</th>
      <th>Actions</th>
    </tr>
  </thead>
  <tbody>
    {}
  </tbody>
</table>"#,
      errored.len(),
      error_rows,
    )
  };

  CLASSIFICATIONS_HTML
    .replace("{{FILTER_INFO}}", &filter_info)
    .replace("{{COUNT}}", &classifications.len().to_string())
    .replace("{{ROWS}}", &rows)
    .replace("{{ADMIN_ACTIONS}}", &admin_actions)
    .replace("{{ERRORS_SECTION}}", &errors_section)
}

fn html_escape(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&#x27;")
}

fn default_ttl_days() -> i64 {
  10
}

#[derive(Deserialize)]
struct ReprojectionParams {
  #[serde(default = "default_ttl_days")]
  ttl_days: i64,
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

  let mut tx = match state.pool.begin().await {
    Ok(tx) => tx,
    Err(e) => {
      error!("Failed to begin transaction: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to begin transaction: {}\n", e),
      );
    }
  };

  let expire_result = DomainExpire {
    domain: params.domain.clone(),
  }
  .expire(&mut tx)
  .await;

  if let Err(e) = tx.commit().await {
    error!("Failed to commit transaction: {}", e);
    return (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("Failed to commit transaction: {}\n", e),
    );
  }

  match expire_result {
    Ok(_) => {
      info!("Expired domain '{}'!", params.domain);
      (
        StatusCode::OK,
        format!("Expired domain successful: {}", params.domain),
      )
    }
    Err(e) => {
      error!("Domain expiration failed: {}", e);
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Domain expiration failed: {}\n", e),
      )
    }
  }
}

#[derive(Deserialize)]
struct RequeueParams {
  domain: String,
  classification_type: String,
}

async fn requeue(
  State(state): State<AppState>,
  Query(params): Query<RequeueParams>,
) -> impl IntoResponse {
  let nats = match &state.nats {
    Some(n) => n.clone(),
    None => {
      return (
        StatusCode::SERVICE_UNAVAILABLE,
        "NATS not configured.  Set --nats-url to enable requeueing.\n"
          .to_string(),
      );
    }
  };

  info!(
    "Requeueing domain '{}' for type '{}'",
    params.domain, params.classification_type
  );

  let errored = match ErroredClassification::find(
    &state.pool,
    Some(&params.classification_type),
  )
  .await
  {
    Ok(e) => e,
    Err(e) => {
      error!("DB error fetching errored classifications: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Database error: {}\n", e),
      );
    }
  };

  if !errored.iter().any(|e| e.domain == params.domain) {
    return (
      StatusCode::NOT_FOUND,
      format!(
        "No error found for domain '{}' with type '{}'\n",
        params.domain, params.classification_type
      ),
    );
  }

  let mut tx = match state.pool.begin().await {
    Ok(tx) => tx,
    Err(e) => {
      error!("Failed to begin transaction: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to begin transaction: {}\n", e),
      );
    }
  };

  if let Err(e) = (DomainRequeue {
    domain: params.domain.clone(),
  })
  .requeue(&mut tx)
  .await
  {
    error!("Failed to insert requeue event: {}", e);
    return (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("Database error: {}\n", e),
    );
  }

  if let Err(e) = tx.commit().await {
    error!("Failed to commit transaction: {}", e);
    return (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("Failed to commit transaction: {}\n", e),
    );
  }

  if let Err(e) = publish_to_nats(&nats, &params.domain).await {
    error!("Failed to publish domain to NATS: {}", e);
    return (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("NATS error: {}\n", e),
    );
  }

  info!(
    "Requeued domain '{}' for type '{}'",
    params.domain, params.classification_type
  );
  (
    StatusCode::OK,
    format!(
      "Requeued domain '{}' for type '{}'\n",
      params.domain, params.classification_type
    ),
  )
}

#[derive(Deserialize)]
struct RequeueTypeParams {
  classification_type: String,
}

async fn requeue_type(
  State(state): State<AppState>,
  Query(params): Query<RequeueTypeParams>,
) -> impl IntoResponse {
  let nats = match &state.nats {
    Some(n) => n.clone(),
    None => {
      return (
        StatusCode::SERVICE_UNAVAILABLE,
        "NATS not configured.  Set --nats-url to enable requeueing.\n"
          .to_string(),
      );
    }
  };

  info!(
    "Requeueing all errored domains for type '{}'",
    params.classification_type
  );

  let errored = match ErroredClassification::find(
    &state.pool,
    Some(&params.classification_type),
  )
  .await
  {
    Ok(e) => e,
    Err(e) => {
      error!("DB error fetching errored classifications: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Database error: {}\n", e),
      );
    }
  };

  if errored.is_empty() {
    return (
      StatusCode::OK,
      format!(
        "No errors found for type '{}'\n",
        params.classification_type
      ),
    );
  }

  let domains: Vec<String> = {
    let mut seen = HashSet::new();
    errored
      .iter()
      .filter(|e| seen.insert(e.domain.clone()))
      .map(|e| e.domain.clone())
      .collect()
  };

  let mut tx = match state.pool.begin().await {
    Ok(tx) => tx,
    Err(e) => {
      error!("Failed to begin transaction: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to begin transaction: {}\n", e),
      );
    }
  };

  for domain in &domains {
    if let Err(e) = (DomainRequeue {
      domain: domain.clone(),
    })
    .requeue(&mut tx)
    .await
    {
      error!("Failed to insert requeue event for '{}': {}", domain, e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Database error: {}\n", e),
      );
    }
  }

  if let Err(e) = tx.commit().await {
    error!("Failed to commit transaction: {}", e);
    return (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("Failed to commit transaction: {}\n", e),
    );
  }

  let mut published = 0usize;
  for domain in &domains {
    match publish_to_nats(&nats, domain).await {
      Ok(_) => published += 1,
      Err(e) => warn!("Failed to publish '{}' to NATS: {}", domain, e),
    }
  }

  info!(
    "Requeued {} domain(s) for type '{}'",
    published, params.classification_type
  );
  (
    StatusCode::OK,
    format!(
      "Requeued {} domain(s) for type '{}'\n",
      published, params.classification_type
    ),
  )
}

async fn requeue_all(State(state): State<AppState>) -> impl IntoResponse {
  let nats = match &state.nats {
    Some(n) => n.clone(),
    None => {
      return (
        StatusCode::SERVICE_UNAVAILABLE,
        "NATS not configured.  Set --nats-url to enable requeueing.\n"
          .to_string(),
      );
    }
  };

  info!("Requeueing all errored domains");

  let errored = match ErroredClassification::find(&state.pool, None).await {
    Ok(e) => e,
    Err(e) => {
      error!("DB error fetching errored classifications: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Database error: {}\n", e),
      );
    }
  };

  if errored.is_empty() {
    return (StatusCode::OK, "No errors found\n".to_string());
  }

  let domains: Vec<String> = {
    let mut seen = HashSet::new();
    errored
      .iter()
      .filter(|e| seen.insert(e.domain.clone()))
      .map(|e| e.domain.clone())
      .collect()
  };

  let mut tx = match state.pool.begin().await {
    Ok(tx) => tx,
    Err(e) => {
      error!("Failed to begin transaction: {}", e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to begin transaction: {}\n", e),
      );
    }
  };

  for domain in &domains {
    if let Err(e) = (DomainRequeue {
      domain: domain.clone(),
    })
    .requeue(&mut tx)
    .await
    {
      error!("Failed to insert requeue event for '{}': {}", domain, e);
      return (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Database error: {}\n", e),
      );
    }
  }

  if let Err(e) = tx.commit().await {
    error!("Failed to commit transaction: {}", e);
    return (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("Failed to commit transaction: {}\n", e),
    );
  }

  let mut published = 0usize;
  for domain in &domains {
    match publish_to_nats(&nats, domain).await {
      Ok(_) => published += 1,
      Err(e) => warn!("Failed to publish '{}' to NATS: {}", domain, e),
    }
  }

  info!("Requeued {} domain(s)", published);
  (
    StatusCode::OK,
    format!("Requeued {} domain(s)\n", published),
  )
}

fn default_classify_confidence() -> f64 {
  1.0
}

fn default_classify_user_id() -> i32 {
  1
}

#[derive(Deserialize)]
struct ClassifyRequest {
  domain: String,
  classification_type: String,
  is_matching_site: bool,
  #[serde(default = "default_classify_confidence")]
  confidence: f64,
  #[serde(default)]
  reasoning: String,
  #[serde(default = "default_ttl_days")]
  ttl_days: i64,
  #[serde(default = "default_classify_user_id")]
  user_id: i32,
}

async fn classify(
  State(state): State<AppState>,
  axum::Json(req): axum::Json<ClassifyRequest>,
) -> impl IntoResponse {
  info!(
    "Applying admin classification for '{}' as '{}' (matching={}, confidence={})",
    req.domain, req.classification_type, req.is_matching_site, req.confidence
  );

  match db::apply_admin_classification(
    &state.pool,
    &req.domain,
    &req.classification_type,
    req.is_matching_site,
    req.confidence,
    &req.reasoning,
    req.user_id,
    req.ttl_days,
  )
  .await
  {
    Ok(source_id) => {
      info!(
        "Admin classification applied for '{}', source_id={}",
        req.domain, source_id
      );
      (
        StatusCode::OK,
        format!("Classification applied: source_id={}\n", source_id),
      )
    }
    Err(e) => {
      error!("Admin classification failed for '{}': {}", req.domain, e);
      (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}\n", e))
    }
  }
}

// ── static asset handlers ─────────────────────────────────────────────────────

async fn static_css() -> impl IntoResponse {
  ([(header::CONTENT_TYPE, "text/css")], CLASSIFICATIONS_CSS)
}

async fn static_js() -> impl IntoResponse {
  (
    [(header::CONTENT_TYPE, "application/javascript")],
    CLASSIFICATIONS_JS,
  )
}

// ── router builders ───────────────────────────────────────────────────────────

fn public_router(state: AppState) -> Router {
  Router::new()
    .route("/blocklist", get(get_blocklist))
    .route("/health", get(health_check))
    .route("/metrics", get(prometheus_metrics))
    .layer(TraceLayer::new_for_http())
    .with_state(state)
}

fn admin_router(state: AppState) -> Router {
  Router::new()
    .route("/classifications", get(get_classifications))
    .route("/classify", post(classify))
    .route("/reprojection", post(reprojection))
    .route("/expire", post(expire))
    .route("/requeue", post(requeue))
    .route("/requeue/type", post(requeue_type))
    .route("/requeue/all", post(requeue_all))
    .route("/static/classifications.css", get(static_css))
    .route("/static/classifications.js", get(static_js))
    .layer(TraceLayer::new_for_http())
    .with_state(state)
}

// ── server startup ────────────────────────────────────────────────────────────

pub async fn run(args: CliArgs) -> Result<(), Box<dyn std::error::Error>> {
  info!("Starting DNS Smart Block Blocklist Server");

  let database_url = if let Some(password_file) = args.database_password_file {
    let password = std::fs::read_to_string(&password_file)
      .map_err(|e| {
        error!(
          "Failed to read database password file '{}': {}",
          password_file.display(),
          e
        );
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

  let sanitized_url = if let Ok(mut url) = url::Url::parse(&database_url) {
    if url.password().is_some() {
      let _ = url.set_password(Some("***"));
    }
    url.to_string()
  } else {
    database_url.clone()
  };

  info!("Connecting to PostgreSQL at {}", sanitized_url);
  let pool = PgPool::connect(&database_url).await?;
  info!("Connected to PostgreSQL successfully");

  let nats = if let Some(ref nats_url) = args.nats_url {
    info!("Connecting to NATS at {}", nats_url);
    match async_nats::connect(nats_url).await {
      Ok(client) => {
        info!("Connected to NATS successfully");
        Some(NatsState {
          client,
          subject: args.nats_subject.clone(),
        })
      }
      Err(e) => {
        error!(
          "Failed to connect to NATS: {}.  Requeue functionality will be disabled.",
          e
        );
        None
      }
    }
  } else {
    info!("No NATS URL configured.  Requeue functionality will be disabled.");
    None
  };

  let state = AppState { pool, nats };

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

  let admin_listener = tokio::net::TcpListener::bind(admin_addr).await?;
  let admin_state = state.clone();
  tokio::spawn(async move {
    if let Err(e) = axum::serve(admin_listener, admin_router(admin_state)).await
    {
      error!("Admin server error: {}", e);
    }
  });

  let public_listener = tokio::net::TcpListener::bind(public_addr).await?;
  axum::serve(public_listener, public_router(state)).await?;

  Ok(())
}
