use crate::error::WorkerError;
use reqwest::redirect::Policy;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, info, warn};

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
}

pub async fn fetch_domain(
  domain: &str,
  timeout_sec: u64,
  max_kb: usize,
) -> Result<(String, u16), WorkerError> {
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
    .build()?;

  let url = if domain.starts_with("http://") || domain.starts_with("https://") {
    domain.to_string()
  } else {
    format!("https://{}", domain)
  };

  let response = client
    .get(&url)
    .header(
      "Accept",
      "text/html,application/xhtml+xml,\
      application/xml;q=0.9,*/*;q=0.8",
    )
    .header("Accept-Language", "en-US,en;q=0.9")
    .send()
    .await?;

  let status = response.status().as_u16();
  info!("HTTP status: {}", status);

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
  Ok((html, status))
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
) -> Result<SiteMetadata, WorkerError> {
  info!("Extracting metadata from HTML");
  let document = Html::parse_document(html);
  let title = text_from_css_selector(&document, "title");
  let description =
    attr_from_css_selector(&document, "meta[name='description']", "content");
  let og_title = text_from_css_selector(&document, "meta[property='og:title']");
  let og_description =
    text_from_css_selector(&document, "meta[property='og:description']");
  let og_site_name =
    text_from_css_selector(&document, "meta[property='og:site_name']");
  let language = text_from_css_selector(&document, "html");
  Ok(SiteMetadata {
    domain: domain.to_string(),
    title,
    description,
    og_title,
    og_description,
    og_site_name,
    language,
    http_status: status,
  })
}
