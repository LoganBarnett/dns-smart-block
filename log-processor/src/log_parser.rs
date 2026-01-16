use crate::Result;
use regex::Regex;
use tracing::debug;

pub struct LogParser {
    domain_patterns: Vec<Regex>,
}

impl LogParser {
    pub fn new() -> Result<Self> {
        // Common DNS log patterns that indicate a successful query
        let patterns = vec![
            // dnsdist query format: "Query from IP:port: domain IN type"
            r"Query from [^\s]+: ([a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?)*) IN",

            // Common DNS log format: "client IP#port (domain)"
            r"client [^\s]+#\d+ \(([a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?)*)\)",

            // Simple format: "query: domain"
            r"query:\s+([a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?)*)",

            // dnsdist with domain followed by query type
            r"\s([a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?)*)\s+(A|AAAA|NS|MX|TXT|CNAME)\s",

            // Systemd journal format with QUERY or DOMAIN field
            r"(?:QUERY|DOMAIN)=([a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?)*)",
        ];

        let domain_patterns = patterns
            .into_iter()
            .map(|p| Regex::new(p))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(Self { domain_patterns })
    }

    /// Parse a log line and extract domain if it represents a DNS query
    pub fn parse_log_line(&self, line: &str) -> Option<String> {
        // Skip empty lines
        if line.trim().is_empty() {
            return None;
        }

        debug!("Parsing log line: {}", line);

        for pattern in &self.domain_patterns {
            if let Some(captures) = pattern.captures(line) {
                if let Some(domain) = captures.get(1) {
                    let domain = domain.as_str();

                    // Validate domain has at least one dot and looks reasonable
                    if domain.contains('.') && is_valid_domain(domain) {
                        debug!("Extracted domain: {}", domain);
                        return Some(domain.to_lowercase());
                    }
                }
            }
        }

        debug!("No domain found in line");
        None
    }
}

impl Default for LogParser {
    fn default() -> Self {
        Self::new().expect("Failed to create default LogParser")
    }
}

/// Validate that a domain looks reasonable
fn is_valid_domain(domain: &str) -> bool {
    // Must have at least one dot
    if !domain.contains('.') {
        return false;
    }

    // Must not start or end with dot or hyphen
    if domain.starts_with('.') || domain.ends_with('.')
        || domain.starts_with('-') || domain.ends_with('-') {
        return false;
    }

    // Must not be too long (max domain length is 253 characters)
    if domain.len() > 253 {
        return false;
    }

    // Should not contain spaces
    if domain.contains(' ') {
        return false;
    }

    // Filter out common localhost/internal domains
    let lower = domain.to_lowercase();
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".localhost")
        || lower.ends_with(".internal") {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dnsdist_query_format() {
        let parser = LogParser::new().unwrap();

        let line = "Query from 192.168.1.100:54321: example.com IN A";
        assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));

        let line = "Query from 10.0.0.5:12345: test.example.org IN AAAA";
        assert_eq!(parser.parse_log_line(line), Some("test.example.org".to_string()));
    }

    #[test]
    fn test_parse_client_format() {
        let parser = LogParser::new().unwrap();

        let line = "client 192.168.1.1#53210 (example.com): query: example.com IN A";
        assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));
    }

    #[test]
    fn test_parse_simple_query_format() {
        let parser = LogParser::new().unwrap();

        let line = "query: example.com";
        assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));
    }

    #[test]
    fn test_parse_domain_with_query_type() {
        let parser = LogParser::new().unwrap();

        let line = "2024-01-16 10:00:00 example.com A query from 192.168.1.1";
        assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));
    }

    #[test]
    fn test_invalid_domains_rejected() {
        let parser = LogParser::new().unwrap();

        // No TLD
        let line = "Query from 192.168.1.100:54321: localhost IN A";
        assert_eq!(parser.parse_log_line(line), None);

        // .local domain
        let line = "Query from 192.168.1.100:54321: myhost.local IN A";
        assert_eq!(parser.parse_log_line(line), None);
    }

    #[test]
    fn test_case_insensitive() {
        let parser = LogParser::new().unwrap();

        let line = "Query from 192.168.1.100:54321: EXAMPLE.COM IN A";
        assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));
    }
}
