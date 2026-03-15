use crate::{ProcessorError, Result};
use async_nats::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DomainMessage {
  pub domain: String,
  pub timestamp: i64,
  /// Resolved IP from the DNS log, when available.  Allows the classifier to
  /// fetch the domain's content directly by IP instead of re-resolving through
  /// the local DNS stack.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub resolved_ip: Option<String>,
}

pub struct QueuePublisher {
  client: Client,
  subject: String,
}

impl QueuePublisher {
  pub async fn new(nats_url: &str, subject: String) -> Result<Self> {
    info!("Connecting to NATS at {}", nats_url);
    let client = async_nats::connect(nats_url)
      .await
      .map_err(|e| ProcessorError::NatsError(e.to_string()))?;
    info!("Connected to NATS successfully");

    Ok(Self { client, subject })
  }

  /// Publish a domain to the queue, with an optional pre-resolved IP address.
  pub async fn publish_domain(
    &self,
    domain: &str,
    resolved_ip: Option<String>,
  ) -> Result<()> {
    let message = DomainMessage {
      domain: domain.to_string(),
      timestamp: chrono::Utc::now().timestamp(),
      resolved_ip,
    };

    let payload = serde_json::to_vec(&message)?;

    debug!("Publishing domain {} to subject {}", domain, self.subject);

    self
      .client
      .publish(self.subject.clone(), payload.into())
      .await
      .map_err(|e| ProcessorError::NatsError(e.to_string()))?;

    info!("Published domain {} to queue", domain);

    Ok(())
  }

  /// Publish multiple domains in a batch (no resolved IPs).
  pub async fn publish_domains(&self, domains: &[String]) -> Result<()> {
    for domain in domains {
      self.publish_domain(domain, None).await?;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_domain_message_serialization() {
    let message = DomainMessage {
      domain: "example.com".to_string(),
      timestamp: 1234567890,
      resolved_ip: Some("1.2.3.4".to_string()),
    };

    let json = serde_json::to_string(&message).unwrap();
    assert!(json.contains("example.com"));
    assert!(json.contains("1234567890"));
    assert!(json.contains("1.2.3.4"));

    let deserialized: DomainMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.domain, "example.com");
    assert_eq!(deserialized.timestamp, 1234567890);
    assert_eq!(deserialized.resolved_ip, Some("1.2.3.4".to_string()));
  }

  #[test]
  fn test_domain_message_without_ip_deserializes() {
    // Messages published before ip support was added must still deserialize.
    let json = r#"{"domain":"example.com","timestamp":1234567890}"#;
    let msg: DomainMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.resolved_ip, None);
  }
}
