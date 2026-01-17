use crate::{ProcessorError, Result};
use async_nats::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DomainMessage {
  pub domain: String,
  pub timestamp: i64,
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

  /// Publish a domain to the queue
  pub async fn publish_domain(&self, domain: &str) -> Result<()> {
    let message = DomainMessage {
      domain: domain.to_string(),
      timestamp: chrono::Utc::now().timestamp(),
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

  /// Publish multiple domains in a batch
  pub async fn publish_domains(&self, domains: &[String]) -> Result<()> {
    for domain in domains {
      self.publish_domain(domain).await?;
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
    };

    let json = serde_json::to_string(&message).unwrap();
    assert!(json.contains("example.com"));
    assert!(json.contains("1234567890"));

    let deserialized: DomainMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.domain, "example.com");
    assert_eq!(deserialized.timestamp, 1234567890);
  }
}
