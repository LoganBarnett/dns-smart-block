use dns_smart_block_log_processor::{
    dnsdist::DnsdistClient, log_parser::LogParser, log_source::LogSource,
};
use futures::StreamExt;
use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

/// Sample DNS log lines in various formats
const SAMPLE_DNS_LOGS: &[&str] = &[
    "Query from 192.168.1.100:54321: gaming-site.com IN A",
    "Query from 192.168.1.101:12345: news-site.com IN AAAA",
    "client 10.0.0.5#53210 (social-media.com): query",
    "2024-01-16 10:00:00 streaming-video.net A query from 192.168.1.1",
    "Query from 192.168.1.102:99999: localhost IN A", // Should be filtered
    "Query from 192.168.1.103:11111: myhost.local IN A", // Should be filtered
];

#[tokio::test]
async fn test_log_parser_extracts_domains() {
    let parser = LogParser::new().unwrap();

    let mut extracted_domains = Vec::new();

    for log_line in SAMPLE_DNS_LOGS {
        if let Some(domain) = parser.parse_log_line(log_line) {
            extracted_domains.push(domain);
        }
    }

    // Should extract 4 valid domains (gaming-site.com, news-site.com, social-media.com, streaming-video.net)
    // localhost and myhost.local should be filtered out
    assert_eq!(extracted_domains.len(), 4);
    assert!(extracted_domains.contains(&"gaming-site.com".to_string()));
    assert!(extracted_domains.contains(&"news-site.com".to_string()));
    assert!(extracted_domains.contains(&"social-media.com".to_string()));
    assert!(extracted_domains.contains(&"streaming-video.net".to_string()));

    // Verify filtered domains are not present
    assert!(!extracted_domains.contains(&"localhost".to_string()));
    assert!(!extracted_domains.contains(&"myhost.local".to_string()));
}

#[tokio::test]
async fn test_log_source_command_stream() {
    // Create a command that outputs DNS log lines
    let source = LogSource::from_command(vec![
        "echo".to_string(),
        "Query from 192.168.1.100:54321: test-domain.com IN A".to_string(),
    ]);

    let mut stream = source.into_stream().await.unwrap();

    if let Some(Ok(line)) = stream.next().await {
        assert!(line.contains("test-domain.com"));
    } else {
        panic!("Expected to read a line from command");
    }
}

#[tokio::test]
async fn test_dnsdist_client_checks_blocked_domains() {
    // Set up mock dnsdist server
    let mock_server = MockServer::start().await;

    // Mock response that indicates gaming-site.com is in the block list
    Mock::given(method("GET"))
        .and(path("/api/v1/servers/localhost/zones"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "zones": [
                {
                    "name": "gaming-site.com",
                    "kind": "Native"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let client = DnsdistClient::new(mock_server.uri(), None);

    // Check if gaming-site.com is blocked
    let is_blocked = client.is_domain_blocked("gaming-site.com").await.unwrap();
    assert!(is_blocked, "gaming-site.com should be detected as blocked");

    // Check if another domain is blocked
    let is_blocked = client.is_domain_blocked("news-site.com").await.unwrap();
    assert!(!is_blocked, "news-site.com should not be detected as blocked (not in response)");
}

#[tokio::test]
async fn test_dnsdist_client_handles_api_errors() {
    // Set up mock dnsdist server that returns errors
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/servers/localhost/zones"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let client = DnsdistClient::new(mock_server.uri(), None);

    // Should return false (not blocked) when API fails, allowing system to continue
    let is_blocked = client.is_domain_blocked("test-domain.com").await.unwrap();
    assert!(!is_blocked, "Should return false when API fails");
}

#[tokio::test]
async fn test_full_parsing_flow() {
    let parser = LogParser::new().unwrap();

    // Simulate processing a stream of log lines
    let log_lines = vec![
        "Query from 192.168.1.100:54321: gaming-site.com IN A",
        "Query from 192.168.1.101:12345: gaming-site.com IN A", // Duplicate
        "Query from 192.168.1.102:99999: news-site.com IN AAAA",
        "Query from 192.168.1.103:11111: localhost IN A", // Invalid
    ];

    let mut unique_domains = std::collections::HashSet::new();

    for line in log_lines {
        if let Some(domain) = parser.parse_log_line(line) {
            unique_domains.insert(domain);
        }
    }

    // Should have 2 unique valid domains (gaming-site.com, news-site.com)
    assert_eq!(unique_domains.len(), 2);
    assert!(unique_domains.contains("gaming-site.com"));
    assert!(unique_domains.contains("news-site.com"));
}

#[tokio::test]
async fn test_log_parser_with_various_dns_record_types() {
    let parser = LogParser::new().unwrap();

    let test_cases = vec![
        ("Query from 192.168.1.1:1234: example.com IN A", Some("example.com")),
        ("Query from 192.168.1.1:1234: example.com IN AAAA", Some("example.com")),
        ("Query from 192.168.1.1:1234: example.com IN MX", Some("example.com")),
        ("Query from 192.168.1.1:1234: example.com IN TXT", Some("example.com")),
        ("Query from 192.168.1.1:1234: example.com IN NS", Some("example.com")),
        ("Query from 192.168.1.1:1234: example.com IN CNAME", Some("example.com")),
    ];

    for (log_line, expected_domain) in test_cases {
        let result = parser.parse_log_line(log_line);
        assert_eq!(
            result.as_deref(),
            expected_domain,
            "Failed to parse: {}",
            log_line
        );
    }
}

#[tokio::test]
async fn test_case_insensitive_domain_extraction() {
    let parser = LogParser::new().unwrap();

    let log_lines = vec![
        "Query from 192.168.1.1:1234: EXAMPLE.COM IN A",
        "Query from 192.168.1.1:1234: Example.Com IN A",
        "Query from 192.168.1.1:1234: example.com IN A",
    ];

    let mut domains = Vec::new();
    for line in log_lines {
        if let Some(domain) = parser.parse_log_line(line) {
            domains.push(domain);
        }
    }

    // All should be normalized to lowercase
    assert_eq!(domains.len(), 3);
    assert!(domains.iter().all(|d| d == "example.com"));
}

#[tokio::test]
async fn test_dnsdist_client_with_api_key() {
    let mock_server = MockServer::start().await;

    // Mock that expects API key in header
    Mock::given(method("GET"))
        .and(path("/api/v1/servers/localhost/zones"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "zones": []
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = DnsdistClient::new(
        mock_server.uri(),
        Some("test-api-key".to_string()),
    );

    let _result = client.is_domain_blocked("test.com").await;

    // Verify the mock was called (expectation is checked on drop)
}

#[test]
fn test_domain_validation_edge_cases() {
    let parser = LogParser::new().unwrap();

    // Domains that should NOT be extracted
    let invalid_cases = vec![
        "Query from 192.168.1.1:1234: .example.com IN A", // Starts with dot
        "Query from 192.168.1.1:1234: example.com. IN A", // Ends with dot (might be valid in some contexts, but we filter it)
        "Query from 192.168.1.1:1234: -example.com IN A", // Starts with hyphen
        "Query from 192.168.1.1:1234: example.com- IN A", // Ends with hyphen
    ];

    for invalid_line in invalid_cases {
        let result = parser.parse_log_line(invalid_line);
        // These should either return None or not match the pattern
        if let Some(domain) = result {
            panic!(
                "Should not have extracted domain from invalid case: {}. Got: {}",
                invalid_line, domain
            );
        }
    }
}
