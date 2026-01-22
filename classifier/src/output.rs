use crate::error::ClassifierErrorType;
use serde::{Deserialize, Serialize};

/// Classification result from the LLM
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Classification {
    pub is_matching_site: bool,
    pub confidence: f64,
}

/// Metadata about the classification process
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClassificationMetadata {
    pub http_status: u16,
    pub model: String,
    pub prompt_hash: String,
}

/// Error information
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorInfo {
    pub error_type: ClassifierErrorType,
    pub message: String,
}

/// Output format for successful classification
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClassificationOutput {
    pub domain: String,
    pub result: String, // "classified"
    pub classification: Classification,
    pub metadata: ClassificationMetadata,
}

/// Output format for errors
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorOutput {
    pub domain: String,
    pub result: String, // "error"
    pub error: ErrorInfo,
    pub metadata: Option<PartialMetadata>,
}

/// Partial metadata available even on error
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PartialMetadata {
    pub model: String,
    pub prompt_hash: String,
}

impl ClassificationOutput {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

impl ErrorOutput {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
