pub mod cli_args;
pub mod error;
pub mod web_classify;

use crate::{error::WorkerError, web_classify::SiteMetadata};
use serde::{Deserialize, Serialize};
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

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct ClassificationResult {
    pub is_matching_site: bool,
    pub confidence: f64,
}

pub async fn classify_with_llm(
    metadata: &SiteMetadata,
    ollama_url: &str,
    model: &str,
    prompt_template: &str,
) -> Result<ClassificationResult, WorkerError> {
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
        .await?;

    if !response.status().is_success() {
        error!("Ollama API returned error: {}", response.status());
        return Err(WorkerError::HttpError(
            response.error_for_status().unwrap_err(),
        ));
    }

    let ollama_response: OllamaResponse = response.json().await?;
    info!("LLM response: {}", ollama_response.response);

    let classification: ClassificationResult =
        serde_json::from_str(&ollama_response.response)?;

    Ok(classification)
}
