use dns_smart_block_log_processor::{
  log_parser::LogParser, log_source::LogSource,
};
use futures::StreamExt;

const BLOCKY_PATTERN: &str =
  r"question_name=(\w(?:[\w-]*\w)?(?:\.\w(?:[\w-]*\w)?)+)\.";
const BLOCKY_FILTER: &str = r"response_type=RESOLVED";

fn blocky_resolved(question_name: &str) -> String {
  format!(
    "[2026-01-16 10:00:00]  INFO queryLog: query resolved \
     answer=A (1.2.3.4) client_ip=192.168.1.1 \
     question_name={question_name}. question_type=A \
     response_code=NOERROR response_reason=RESOLVED (tcp+udp:1.1.1.1) \
     response_type=RESOLVED"
  )
}

fn blocky_cached(question_name: &str) -> String {
  format!(
    "[2026-01-16 10:00:00]  INFO queryLog: query resolved \
     question_name={question_name}. question_type=A \
     response_code=NOERROR response_type=CACHED"
  )
}

fn blocky_blocked(question_name: &str) -> String {
  format!(
    "[2026-01-16 10:00:00]  INFO queryLog: query resolved \
     answer=A (0.0.0.0) client_ip=192.168.1.1 \
     question_name={question_name}. question_type=A \
     response_code=NOERROR response_reason=BLOCKED response_type=BLOCKED"
  )
}

#[tokio::test]
async fn test_log_parser_extracts_resolved_domains_only() {
  let parser =
    LogParser::new(BLOCKY_PATTERN, 1, Some(BLOCKY_FILTER), None, 1).unwrap();

  let log_lines = [
    blocky_resolved("gaming-site.com"),
    blocky_resolved("news-site.com"),
    blocky_cached("social-media.com"), // should be filtered out
    blocky_blocked("blocked-site.com"), // should be filtered out
    blocky_resolved("myhost.local"),   // should be filtered by is_valid_domain
  ];

  let mut extracted: Vec<String> = log_lines
    .iter()
    .filter_map(|line| parser.parse_log_line(line).map(|p| p.domain))
    .collect();
  extracted.sort();

  assert_eq!(extracted, vec!["gaming-site.com", "news-site.com"]);
}

#[tokio::test]
async fn test_log_source_command_stream() {
  let source = LogSource::from_command(vec![
    "echo".to_string(),
    blocky_resolved("test-domain.com"),
  ]);

  let mut stream = source.into_stream().await.unwrap();

  if let Some(Ok(line)) = stream.next().await {
    assert!(line.contains("test-domain.com"));
  } else {
    panic!("Expected to read a line from command");
  }
}

#[tokio::test]
async fn test_full_parsing_flow_deduplication() {
  let parser =
    LogParser::new(BLOCKY_PATTERN, 1, Some(BLOCKY_FILTER), None, 1).unwrap();

  let log_lines = [
    blocky_resolved("gaming-site.com"),
    blocky_resolved("gaming-site.com"), // duplicate
    blocky_resolved("news-site.com"),
    blocky_resolved("localhost"), // invalid, no dot after filtering
  ];

  let mut unique_domains = std::collections::HashSet::new();
  for line in &log_lines {
    if let Some(parsed) = parser.parse_log_line(line) {
      unique_domains.insert(parsed.domain);
    }
  }

  assert_eq!(unique_domains.len(), 2);
  assert!(unique_domains.contains("gaming-site.com"));
  assert!(unique_domains.contains("news-site.com"));
}

#[tokio::test]
async fn test_log_parser_with_various_dns_record_types() {
  let parser =
    LogParser::new(BLOCKY_PATTERN, 1, Some(BLOCKY_FILTER), None, 1).unwrap();

  for record_type in ["A", "AAAA", "MX", "TXT", "NS", "CNAME"] {
    let line = format!(
      "[2026-01-16 10:00:00]  INFO queryLog: query resolved \
       answer={record_type} (1.2.3.4) client_ip=192.168.1.1 \
       question_name=example.com. question_type={record_type} \
       response_code=NOERROR response_type=RESOLVED"
    );
    assert_eq!(
      parser.parse_log_line(&line).map(|p| p.domain),
      Some("example.com".to_string()),
      "Failed for record type {record_type}"
    );
  }
}

#[tokio::test]
async fn test_case_insensitive_domain_extraction() {
  let parser =
    LogParser::new(BLOCKY_PATTERN, 1, Some(BLOCKY_FILTER), None, 1).unwrap();

  let domains =
    ["EXAMPLE.COM", "Example.Com", "example.com"].map(blocky_resolved);

  let extracted: Vec<String> = domains
    .iter()
    .filter_map(|line| parser.parse_log_line(line).map(|p| p.domain))
    .collect();

  assert_eq!(extracted.len(), 3);
  assert!(extracted.iter().all(|d| d == "example.com"));
}

#[test]
fn test_domain_validation_edge_cases() {
  let parser =
    LogParser::new(BLOCKY_PATTERN, 1, Some(BLOCKY_FILTER), None, 1).unwrap();

  // The Blocky pattern requires a trailing dot, so domains that end with a
  // hyphen or start with a hyphen won't match the regex at all.  Domains
  // starting with a dot also won't match.  These are tested to confirm None.
  let invalid_cases = [
    blocky_resolved("-example.com"),
    blocky_resolved("example-.com"),
  ];

  for line in &invalid_cases {
    assert_eq!(
      parser.parse_log_line(line),
      None,
      "Should not have extracted domain from: {line}"
    );
  }
}
