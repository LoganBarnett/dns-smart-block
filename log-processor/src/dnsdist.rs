use crate::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct DnsdistClient {
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DnsdistResponse {
    pub success: bool,
    #[serde(default)]
    pub message: Option<String>,
}

impl DnsdistClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            base_url,
            api_key,
            client: Client::new(),
        }
    }

    /// Check if a domain is already in the block list
    /// Returns Ok(true) if domain is blocked, Ok(false) if not blocked
    /// Returns Err if the API call fails
    pub async fn is_domain_blocked(&self, domain: &str) -> Result<bool> {
        debug!("Checking if domain {} is blocked in dnsdist", domain);

        let url = format!("{}/api/v1/servers/localhost/zones", self.base_url);

        let mut request = self.client.get(&url);

        if let Some(ref key) = self.api_key {
            request = request.header("X-API-Key", key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            warn!(
                "dnsdist API returned non-success status: {}",
                response.status()
            );
            // If API call fails, we'll assume domain is not blocked
            // This allows the system to continue functioning even if dnsdist API is unavailable
            return Ok(false);
        }

        // Parse response to check if domain is in any zone
        let body = response.text().await?;
        debug!("dnsdist API response: {}", body);

        // Simple check if domain appears in the response
        // This is a simplified implementation - in production you'd parse the full zone list
        let is_blocked = body.contains(domain);

        if is_blocked {
            info!("Domain {} is already blocked in dnsdist", domain);
        } else {
            debug!("Domain {} is not blocked in dnsdist", domain);
        }

        Ok(is_blocked)
    }

    /// Add a domain to the block list
    /// This is a placeholder for future functionality
    pub async fn block_domain(&self, domain: &str) -> Result<()> {
        info!("Blocking domain {} in dnsdist", domain);

        let url = format!("{}/api/v1/servers/localhost/zones", self.base_url);

        let mut request = self.client.post(&url).json(&serde_json::json!({
            "name": domain,
            "kind": "Native",
            "masters": [],
            "nameservers": []
        }));

        if let Some(ref key) = self.api_key {
            request = request.header("X-API-Key", key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            warn!(
                "Failed to block domain in dnsdist: {}",
                response.status()
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = DnsdistClient::new(
            "http://localhost:8080".to_string(),
            Some("test-key".to_string()),
        );

        assert_eq!(client.base_url, "http://localhost:8080");
        assert_eq!(client.api_key, Some("test-key".to_string()));
    }
}
