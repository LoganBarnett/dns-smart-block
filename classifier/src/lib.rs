pub mod cli_args;
pub mod error;
pub mod output;
pub mod web_classify;

use crate::{error::ClassifierError, output::Classification, web_classify::SiteMetadata};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{error, info};

#[derive(Serialize, Debug, Clone)]
pub struct OllamaRequest {
    pub model: String,
    pub prompt: String,
    pub format: String,
    pub stream: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OllamaResponse {
    pub response: String,
}

/// Compute SHA256 hash of prompt content
pub fn compute_prompt_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

pub async fn classify_with_llm(
    metadata: &SiteMetadata,
    ollama_url: &str,
    model: &str,
    prompt_template: &str,
) -> Result<Classification, ClassifierError> {
    info!("Classifying domain with LLM");

    let metadata_json = serde_json::to_string(metadata)?;
    let prompt = prompt_template.replace("{{INPUT_JSON}}", &metadata_json);

    let ollama_request = OllamaRequest {
        model: model.to_string(),
        prompt,
        format: "json".to_string(),
        stream: false,
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/generate", ollama_url))
        .json(&ollama_request)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                error!("Ollama API timeout");
            } else if e.is_connect() {
                error!("Failed to connect to Ollama API");
            }
            e
        })?;

    if !response.status().is_success() {
        error!("Ollama API returned error: {}", response.status());
        return Err(ClassifierError::HttpError(
            response.error_for_status().unwrap_err(),
        ));
    }

    let ollama_response: OllamaResponse = response.json().await.map_err(|e| {
        error!("Failed to parse Ollama response");
        e
    })?;

    info!("LLM response: {}", ollama_response.response);

    let classification: Classification =
        serde_json::from_str(&ollama_response.response).map_err(|e| {
            error!("Failed to parse classification from LLM output");
            ClassifierError::JsonError(e)
        })?;

    Ok(classification)
}
