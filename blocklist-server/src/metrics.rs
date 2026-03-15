use crate::db::MetricsStats;
use lazy_static::lazy_static;
use prometheus::{
  GaugeVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts,
  register_gauge_vec, register_int_counter, register_int_counter_vec,
  register_int_gauge, register_int_gauge_vec,
};

lazy_static! {
  // Request tracking metrics.
  pub static ref BLOCKLIST_REQUESTS_TOTAL: IntCounterVec = register_int_counter_vec!(
    Opts::new("dns_smart_block_blocklist_requests_total", "Total number of blocklist requests"),
    &["classification_type", "status"]
  ).unwrap();

  pub static ref BLOCKLIST_LAST_REQUEST_TIMESTAMP: GaugeVec = register_gauge_vec!(
    Opts::new(
      "dns_smart_block_last_blocklist_request_timestamp_seconds",
      "Unix timestamp of the most recent successful blocklist request by type",
    ),
    &["classification_type"]
  ).unwrap();

  pub static ref BLOCKLIST_DOMAINS_COUNT: IntGauge = register_int_gauge!(
    "dns_smart_block_blocklist_domains_total",
    "Total number of blocked domains across all classifications"
  ).unwrap();

  pub static ref HEALTH_CHECK_REQUESTS_TOTAL: IntCounter = register_int_counter!(
    "dns_smart_block_health_check_requests_total",
    "Total number of health check requests"
  ).unwrap();

  pub static ref METRICS_REQUESTS_TOTAL: IntCounter = register_int_counter!(
    "dns_smart_block_metrics_requests_total",
    "Total number of metrics requests"
  ).unwrap();

  // Database state metrics (gauges).
  pub static ref DOMAINS_CLASSIFIED_CURRENT: IntGaugeVec = register_int_gauge_vec!(
    Opts::new("dns_smart_block_domains_classified", "Currently valid classified domains by type"),
    &["classification_type"]
  ).unwrap();

  pub static ref DOMAINS_CLASSIFIED_POSITIVE: IntGaugeVec = register_int_gauge_vec!(
    Opts::new("dns_smart_block_domains_classified_positive", "Currently valid positive classifications by type"),
    &["classification_type"]
  ).unwrap();

  pub static ref DOMAINS_CLASSIFIED_NEGATIVE: IntGaugeVec = register_int_gauge_vec!(
    Opts::new("dns_smart_block_domains_classified_negative", "Currently valid negative classifications by type"),
    &["classification_type"]
  ).unwrap();

  pub static ref DOMAINS_CLASSIFIED_TOTAL_CURRENT: IntGauge = register_int_gauge!(
    "dns_smart_block_domains_classified_total",
    "Total currently valid classified domains (all types)"
  ).unwrap();

  pub static ref DOMAINS_CLASSIFIED_POSITIVE_TOTAL: IntGauge = register_int_gauge!(
    "dns_smart_block_domains_classified_positive_total",
    "Total currently valid positive classifications"
  ).unwrap();

  pub static ref DOMAINS_CLASSIFIED_NEGATIVE_TOTAL: IntGauge = register_int_gauge!(
    "dns_smart_block_domains_classified_negative_total",
    "Total currently valid negative classifications"
  ).unwrap();

  pub static ref DOMAINS_SEEN_TOTAL: IntGauge = register_int_gauge!(
    "dns_smart_block_domains_seen",
    "Total unique domains ever seen"
  ).unwrap();

  pub static ref EVENTS_BY_ACTION: IntGaugeVec = register_int_gauge_vec!(
    Opts::new("dns_smart_block_events", "Count of classification events by action"),
    &["action"]
  ).unwrap();

  // Cumulative metrics (counters represented as gauges for total counts).
  pub static ref CLASSIFICATIONS_CREATED_TOTAL: IntGaugeVec = register_int_gauge_vec!(
    Opts::new("dns_smart_block_classifications_total", "Total classifications ever created by type"),
    &["classification_type"]
  ).unwrap();

  pub static ref CLASSIFICATIONS_CREATED_ALL_TOTAL: IntGauge = register_int_gauge!(
    "dns_smart_block_classifications_all_total",
    "Total classifications ever created (all types)"
  ).unwrap();

  // Recent classification event metrics (time-windowed).
  pub static ref RECENT_CLASSIFICATIONS_5M: IntGaugeVec = register_int_gauge_vec!(
    Opts::new("dns_smart_block_recent_classifications_5m", "Classified events in the last 5 minutes by type"),
    &["classification_type"]
  ).unwrap();
}

/// Update all database-derived gauges from a freshly fetched `MetricsStats`.
pub fn update_from_stats(stats: &MetricsStats) {
  for (classification_type, count) in &stats.current_classifications_by_type {
    DOMAINS_CLASSIFIED_CURRENT
      .with_label_values(&[classification_type])
      .set(*count);
  }

  for (classification_type, count) in &stats.current_positive_by_type {
    DOMAINS_CLASSIFIED_POSITIVE
      .with_label_values(&[classification_type])
      .set(*count);
  }

  for (classification_type, count) in &stats.current_negative_by_type {
    DOMAINS_CLASSIFIED_NEGATIVE
      .with_label_values(&[classification_type])
      .set(*count);
  }

  DOMAINS_CLASSIFIED_TOTAL_CURRENT.set(stats.current_classifications_total);
  DOMAINS_CLASSIFIED_POSITIVE_TOTAL.set(stats.current_positive_total);
  DOMAINS_CLASSIFIED_NEGATIVE_TOTAL.set(stats.current_negative_total);
  DOMAINS_SEEN_TOTAL.set(stats.domains_seen_total);

  for (action, count) in &stats.events_by_action {
    EVENTS_BY_ACTION.with_label_values(&[action]).set(*count);
  }

  for (classification_type, count) in &stats.classifications_created_by_type {
    CLASSIFICATIONS_CREATED_TOTAL
      .with_label_values(&[classification_type])
      .set(*count);
  }

  CLASSIFICATIONS_CREATED_ALL_TOTAL.set(stats.classifications_created_total);

  for (classification_type, count) in &stats.recent_classified_by_type {
    RECENT_CLASSIFICATIONS_5M
      .with_label_values(&[classification_type])
      .set(*count);
  }
}
