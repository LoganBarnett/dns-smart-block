use crate::Result;
use regex::Regex;
use tracing::debug;

pub struct LogParser {
  domain_pattern: Regex,
  capture_group: usize,
  line_filter: Option<Regex>,
}

impl LogParser {
  pub fn new(
    domain_pattern: &str,
    capture_group: usize,
    line_filter: Option<&str>,
  ) -> Result<Self> {
    let domain_pattern = Regex::new(domain_pattern)?;
    let line_filter = line_filter.map(Regex::new).transpose()?;
    Ok(Self {
      domain_pattern,
      capture_group,
      line_filter,
    })
  }

  /// Parse a log line and extract a domain if it passes the line filter and
  /// the domain pattern matches.
  pub fn parse_log_line(&self, line: &str) -> Option<String> {
    if line.trim().is_empty() {
      return None;
    }

    debug!("Parsing log line: {}", line);

    if let Some(ref filter) = self.line_filter {
      if !filter.is_match(line) {
        debug!("Line filter did not match, skipping");
        return None;
      }
    }

    if let Some(captures) = self.domain_pattern.captures(line) {
      if let Some(domain) = captures.get(self.capture_group) {
        let domain = domain.as_str();
        if is_valid_domain(domain) {
          debug!("Extracted domain: {}", domain);
          return Some(domain.to_lowercase());
        }
      }
    }

    debug!("No domain found in line");
    None
  }
}

/// Validate that a domain looks reasonable.
fn is_valid_domain(domain: &str) -> bool {
  if !domain.contains('.') {
    return false;
  }

  if domain.starts_with('.')
    || domain.ends_with('.')
    || domain.starts_with('-')
    || domain.ends_with('-')
  {
    return false;
  }

  if domain.len() > 253 {
    return false;
  }

  if domain.contains(' ') {
    return false;
  }

  let lower = domain.to_lowercase();
  if lower == "localhost"
    || lower.ends_with(".local")
    || lower.ends_with(".localhost")
    || lower.ends_with(".internal")
  {
    return false;
  }

  true
}

#[cfg(test)]
mod tests {
  use super::*;

  // Blocky pattern used across unit tests.
  const BLOCKY_PATTERN: &str =
    r"question_name=(\w(?:[\w-]*\w)?(?:\.\w(?:[\w-]*\w)?)+)\.";
  const BLOCKY_FILTER: &str = r"response_type=RESOLVED";

  fn blocky_parser() -> LogParser {
    LogParser::new(BLOCKY_PATTERN, 1, Some(BLOCKY_FILTER)).unwrap()
  }

  #[test]
  fn test_parse_blocky_resolved() {
    let parser = blocky_parser();

    let line = "[2026-02-04 20:33:21]  INFO queryLog: query resolved \
      answer=A (13.107.213.69) client_ip=127.0.0.1 \
      question_name=minecraft.net. question_type=A \
      response_code=NOERROR response_reason=RESOLVED (tcp+udp:1.1.1.1) \
      response_type=RESOLVED";
    assert_eq!(
      parser.parse_log_line(line),
      Some("minecraft.net".to_string())
    );
  }

  #[test]
  fn test_blocky_line_filter_excludes_blocked() {
    let parser = blocky_parser();

    // Blocky returns NOERROR for blocked queries too (0.0.0.0 sinkhole) — the
    // line filter on response_type=RESOLVED is what excludes these.
    let line = "[2026-02-04 20:33:21]  INFO queryLog: query resolved \
      answer=A (0.0.0.0) client_ip=192.168.1.1 \
      question_name=adsite.com. question_type=A \
      response_code=NOERROR response_reason=BLOCKED response_type=BLOCKED";
    assert_eq!(parser.parse_log_line(line), None);
  }

  #[test]
  fn test_blocky_line_filter_excludes_cached() {
    let parser = blocky_parser();

    let line = "[2026-02-04 20:33:21]  INFO queryLog: query resolved \
      question_name=example.com. question_type=A \
      response_code=NOERROR response_type=CACHED";
    assert_eq!(parser.parse_log_line(line), None);
  }

  #[test]
  fn test_blocky_line_filter_excludes_nxdomain_suffix_spam() {
    let parser = blocky_parser();

    // These are the .proton/.proton.proton suffix entries that caused the
    // domain-suffix-appending bug.  They are always CACHED/NXDOMAIN and never
    // RESOLVED, so the line filter drops them cleanly.
    let line = "[2026-02-04 20:33:21]  INFO queryLog: query resolved \
      question_name=history.google.com.proton.proton. question_type=A \
      response_code=NXDOMAIN response_type=CACHED";
    assert_eq!(parser.parse_log_line(line), None);
  }

  #[test]
  fn test_no_line_filter() {
    // Without a line filter every line with a matching domain pattern passes.
    let parser = LogParser::new(BLOCKY_PATTERN, 1, None).unwrap();

    let line = "question_name=example.com. response_type=CACHED";
    assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));
  }

  #[test]
  fn test_custom_capture_group() {
    // Pattern where group 1 is a prefix we don't want and group 2 is the
    // domain — verifies capture_group selection works.
    let parser = LogParser::new(
      r"(prefix\.)([a-zA-Z0-9][a-zA-Z0-9-]*\.[a-zA-Z0-9][a-zA-Z0-9-]*)",
      2,
      None,
    )
    .unwrap();

    let line = "prefix.example.com";
    assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));
  }

  #[test]
  fn test_invalid_domains_rejected() {
    let parser = blocky_parser();

    // Local domains should not pass is_valid_domain.
    let local = "[2026-02-04 20:33:21]  INFO queryLog: query resolved \
      question_name=myhost.local. response_type=RESOLVED";
    assert_eq!(parser.parse_log_line(local), None);
  }

  #[test]
  fn test_case_normalised_to_lowercase() {
    let parser = blocky_parser();

    let line = "[2026-02-04 20:33:21]  INFO queryLog: query resolved \
      question_name=EXAMPLE.COM. response_type=RESOLVED";
    assert_eq!(parser.parse_log_line(line), Some("example.com".to_string()));
  }

  #[test]
  fn test_empty_line_returns_none() {
    let parser = blocky_parser();
    assert_eq!(parser.parse_log_line(""), None);
    assert_eq!(parser.parse_log_line("   "), None);
  }
}
