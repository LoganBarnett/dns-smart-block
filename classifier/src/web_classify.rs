use crate::error::ClassifierError;
use reqwest::redirect::Policy;
use scraper::{Html, Selector};
use serde::Serialize;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Serialize, Debug)]
pub struct SiteMetadata {
  pub domain: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub og_title: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub og_description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub og_site_name: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub language: Option<String>,
  pub http_status: u16,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub fetch_error: Option<String>,
}

impl SiteMetadata {
  /// Create minimal metadata for a domain when HTTP fetch fails
  pub fn from_fetch_error(domain: &str, error: &str) -> Self {
    Self {
      domain: domain.to_string(),
      title: None,
      description: None,
      og_title: None,
      og_description: None,
      og_site_name: None,
      language: None,
      http_status: 0,
      fetch_error: Some(error.to_string()),
    }
  }
}

pub async fn fetch_domain(
  domain: &str,
  timeout_sec: u64,
  max_kb: usize,
) -> Result<(String, u16), ClassifierError> {
  info!("Fetching domain: {}", domain);

  let client = reqwest::Client::builder()
    .redirect(Policy::limited(10))
    .timeout(Duration::from_secs(timeout_sec))
    .user_agent(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
       AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 \
       Safari/605.1.15",
    )
    .gzip(true)
    .danger_accept_invalid_certs(true)
    .build()?;

  let url = if domain.starts_with("http://") || domain.starts_with("https://") {
    domain.to_string()
  } else {
    format!("https://{}", domain)
  };

  // Retry logic with exponential backoff: 3 attempts with 500ms, 1s, 2s delays
  let max_attempts = 3;
  let mut last_error = None;

  for attempt in 0..max_attempts {
    if attempt > 0 {
      let delay_ms = 500 * (1 << (attempt - 1)); // 500ms, 1000ms, 2000ms
      warn!("Retry attempt {} after {}ms delay", attempt + 1, delay_ms);
      tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    match client
      .get(&url)
      .header(
        "Accept",
        "text/html,application/xhtml+xml,\
        application/xml;q=0.9,*/*;q=0.8",
      )
      .header("Accept-Language", "en-US,en;q=0.9")
      .send()
      .await
    {
      Ok(response) => {
        let status = response.status().as_u16();
        info!("HTTP status: {} (attempt {})", status, attempt + 1);

        let max_bytes = max_kb * 1024;
        let body_bytes = response.bytes().await?;

        let body = if body_bytes.len() > max_bytes {
          info!(
            "Truncating response from {} bytes to {} KB",
            body_bytes.len(),
            max_kb
          );
          &body_bytes[..max_bytes]
        } else {
          &body_bytes[..]
        };

        let html = String::from_utf8_lossy(body).to_string();
        return Ok((html, status));
      }
      Err(e) => {
        warn!("HTTP request failed on attempt {}: {}", attempt + 1, e);
        last_error = Some(e);
      }
    }
  }

  // All retries exhausted, return the last error
  Err(last_error.unwrap().into())
}

pub fn attr_from_css_selector(
  document: &Html,
  css_selector: &str,
  attr: &str,
) -> Option<String> {
  Selector::parse(css_selector)
    .ok()
    .and_then(|sel| document.select(&sel).next())
    .and_then(|el| el.value().attr(attr))
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

pub fn text_from_css_selector(
  document: &Html,
  css_selector: &str,
) -> Option<String> {
  Selector::parse(css_selector)
    .ok()
    .and_then(|sel| document.select(&sel).next())
    .map(|el| el.text().collect::<String>().trim().to_string())
    .filter(|s| !s.is_empty())
}

pub fn extract_metadata(
  domain: &str,
  html: &str,
  status: u16,
) -> Result<SiteMetadata, ClassifierError> {
  info!("Extracting metadata from HTML");
  let document = Html::parse_document(html);
  let title = text_from_css_selector(&document, "title");
  let description =
    attr_from_css_selector(&document, "meta[name='description']", "content");
  let og_title =
    attr_from_css_selector(&document, "meta[property='og:title']", "content");
  let og_description = attr_from_css_selector(
    &document,
    "meta[property='og:description']",
    "content",
  );
  let og_site_name = attr_from_css_selector(
    &document,
    "meta[property='og:site_name']",
    "content",
  );
  let language = attr_from_css_selector(&document, "html", "lang");
  Ok(SiteMetadata {
    domain: domain.to_string(),
    title,
    description,
    og_title,
    og_description,
    og_site_name,
    language,
    http_status: status,
    fetch_error: None,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_extract_metadata_all_fields_present() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="en">
        <head>
          <title>Test Page Title</title>
          <meta name="description" content="This is a test description">
          <meta property="og:title" content="OG Test Title">
          <meta property="og:description" content="OG Test Description">
          <meta property="og:site_name" content="Test Site">
        </head>
        <body>
          <h1>Hello World</h1>
        </body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.domain, "example.com");
    assert_eq!(result.title, Some("Test Page Title".to_string()));
    assert_eq!(
      result.description,
      Some("This is a test description".to_string())
    );
    assert_eq!(result.og_title, Some("OG Test Title".to_string()));
    assert_eq!(
      result.og_description,
      Some("OG Test Description".to_string())
    );
    assert_eq!(result.og_site_name, Some("Test Site".to_string()));
    assert_eq!(result.language, Some("en".to_string()));
    assert_eq!(result.http_status, 200);
    assert_eq!(result.fetch_error, None);
  }

  #[test]
  fn test_extract_metadata_missing_title() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="en">
        <head>
          <meta name="description" content="Description without title">
        </head>
        <body>
          <h1>No Title Tag</h1>
        </body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.title, None);
    assert_eq!(
      result.description,
      Some("Description without title".to_string())
    );
  }

  #[test]
  fn test_extract_metadata_missing_description() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="en">
        <head>
          <title>Title without Description</title>
        </head>
        <body>
          <p>No description meta tag</p>
        </body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.title, Some("Title without Description".to_string()));
    assert_eq!(result.description, None);
  }

  #[test]
  fn test_extract_metadata_missing_og_tags() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="en">
        <head>
          <title>Basic Page</title>
          <meta name="description" content="Basic description">
        </head>
        <body>
          <p>No Open Graph tags</p>
        </body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.title, Some("Basic Page".to_string()));
    assert_eq!(result.description, Some("Basic description".to_string()));
    assert_eq!(result.og_title, None);
    assert_eq!(result.og_description, None);
    assert_eq!(result.og_site_name, None);
  }

  #[test]
  fn test_extract_metadata_missing_language() {
    let html = r#"
      <!DOCTYPE html>
      <html>
        <head>
          <title>No Language Specified</title>
        </head>
        <body>
          <p>HTML tag has no lang attribute</p>
        </body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.language, None);
    assert_eq!(result.title, Some("No Language Specified".to_string()));
  }

  #[test]
  fn test_extract_metadata_all_fields_missing() {
    let html = r#"
      <!DOCTYPE html>
      <html>
        <head>
        </head>
        <body>
          <p>Minimal HTML</p>
        </body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 404).unwrap();

    assert_eq!(result.domain, "example.com");
    assert_eq!(result.title, None);
    assert_eq!(result.description, None);
    assert_eq!(result.og_title, None);
    assert_eq!(result.og_description, None);
    assert_eq!(result.og_site_name, None);
    assert_eq!(result.language, None);
    assert_eq!(result.http_status, 404);
    assert_eq!(result.fetch_error, None);
  }

  #[test]
  fn test_extract_metadata_whitespace_trimming() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="  en  ">
        <head>
          <title>  Title with spaces  </title>
          <meta name="description" content="  Description with spaces  ">
        </head>
        <body></body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.title, Some("Title with spaces".to_string()));
    assert_eq!(
      result.description,
      Some("Description with spaces".to_string())
    );
    assert_eq!(result.language, Some("en".to_string()));
  }

  #[test]
  fn test_extract_metadata_empty_content() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="">
        <head>
          <title></title>
          <meta name="description" content="">
          <meta property="og:title" content="">
        </head>
        <body></body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.title, None);
    assert_eq!(result.description, None);
    assert_eq!(result.og_title, None);
    assert_eq!(result.language, None);
  }

  #[test]
  fn test_extract_metadata_partial_og_tags() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="es">
        <head>
          <title>Partial OG</title>
          <meta property="og:title" content="OG Title Only">
        </head>
        <body></body>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.og_title, Some("OG Title Only".to_string()));
    assert_eq!(result.og_description, None);
    assert_eq!(result.og_site_name, None);
    assert_eq!(result.language, Some("es".to_string()));
  }

  #[test]
  fn test_extract_metadata_various_language_codes() {
    let test_cases = vec![
      ("en", "en"),
      ("en-US", "en-US"),
      ("fr-FR", "fr-FR"),
      ("zh-CN", "zh-CN"),
      ("ja", "ja"),
    ];

    for (input_lang, expected_lang) in test_cases {
      let html = format!(
        r#"
        <!DOCTYPE html>
        <html lang="{}">
          <head><title>Test</title></head>
          <body></body>
        </html>
      "#,
        input_lang
      );

      let result = extract_metadata("example.com", &html, 200).unwrap();
      assert_eq!(
        result.language,
        Some(expected_lang.to_string()),
        "Failed for language: {}",
        input_lang
      );
    }
  }

  #[test]
  fn test_extract_metadata_malformed_html() {
    let html = r#"
      <html><head><title>Unclosed tags
      <meta name="description" content="Missing closing quote>
      <body>Random content
    "#;

    let result = extract_metadata("example.com", html, 200);

    assert!(result.is_ok(), "Should handle malformed HTML gracefully");
    let metadata = result.unwrap();
    assert_eq!(metadata.domain, "example.com");
  }

  #[test]
  fn test_extract_metadata_http_status_codes() {
    let html = r#"<html><head><title>Test</title></head></html>"#;

    let status_codes = vec![200, 301, 302, 404, 500, 503];

    for status in status_codes {
      let result = extract_metadata("example.com", html, status).unwrap();
      assert_eq!(
        result.http_status, status,
        "HTTP status should be preserved"
      );
    }
  }

  #[test]
  fn test_site_metadata_from_fetch_error() {
    let metadata =
      SiteMetadata::from_fetch_error("example.com", "Connection timeout");

    assert_eq!(metadata.domain, "example.com");
    assert_eq!(metadata.title, None);
    assert_eq!(metadata.description, None);
    assert_eq!(metadata.og_title, None);
    assert_eq!(metadata.og_description, None);
    assert_eq!(metadata.og_site_name, None);
    assert_eq!(metadata.language, None);
    assert_eq!(metadata.http_status, 0);
    assert_eq!(metadata.fetch_error, Some("Connection timeout".to_string()));
  }

  #[test]
  fn test_extract_metadata_og_content_attribute() {
    let html = r#"
      <!DOCTYPE html>
      <html>
        <head>
          <meta property="og:description" content="OG Description via content attribute">
          <meta property="og:site_name" content="Site Name via content attribute">
        </head>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(
      result.og_description,
      Some("OG Description via content attribute".to_string())
    );
    assert_eq!(
      result.og_site_name,
      Some("Site Name via content attribute".to_string())
    );
  }

  #[test]
  fn test_extract_metadata_complex_title_with_entities() {
    let html = r#"
      <!DOCTYPE html>
      <html lang="en">
        <head>
          <title>Test &amp; Example - Site Name</title>
        </head>
      </html>
    "#;

    let result = extract_metadata("example.com", html, 200).unwrap();

    assert_eq!(result.title, Some("Test & Example - Site Name".to_string()));
  }
}
