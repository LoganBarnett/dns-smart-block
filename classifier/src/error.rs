use serde::{Deserialize, Serialize};
use std::fmt;
use strum::EnumString;
use thiserror::Error;

/// Semantic error types that provide context about where/when a failure occurred
#[derive(Debug, EnumString, Serialize, Deserialize, Clone, PartialEq)]
#[strum(serialize_all = "PascalCase")]
#[serde(rename_all = "PascalCase")]
pub enum ClassifierErrorType {
    PromptFileReadError,
    DomainFetchError,
    DomainFetchTimeoutError,
    HtmlParseError,
    OllamaApiConnectionError,
    OllamaApiTimeoutError,
    OllamaApiError,
    OllamaResponseParseError,
    ClassificationParseError,
    MetadataSerializationError,
}

impl fmt::Display for ClassifierErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PromptFileReadError => {
                write!(f, "PromptFileReadError")
            }
            Self::DomainFetchError => write!(f, "DomainFetchError"),
            Self::DomainFetchTimeoutError => {
                write!(f, "DomainFetchTimeoutError")
            }
            Self::HtmlParseError => write!(f, "HtmlParseError"),
            Self::OllamaApiConnectionError => {
                write!(f, "OllamaApiConnectionError")
            }
            Self::OllamaApiTimeoutError => write!(f, "OllamaApiTimeoutError"),
            Self::OllamaApiError => write!(f, "OllamaApiError"),
            Self::OllamaResponseParseError => {
                write!(f, "OllamaResponseParseError")
            }
            Self::ClassificationParseError => {
                write!(f, "ClassificationParseError")
            }
            Self::MetadataSerializationError => {
                write!(f, "MetadataSerializationError")
            }
        }
    }
}

/// Internal error type with detailed context
#[derive(Error, Debug)]
pub enum ClassifierError {
    #[error("Prompt file read error: {0}")]
    PromptFileReadError(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
}

impl ClassifierError {
    /// Convert internal error to semantic error type
    pub fn to_error_type(&self) -> ClassifierErrorType {
        match self {
            ClassifierError::PromptFileReadError(_) => {
                ClassifierErrorType::PromptFileReadError
            }
            ClassifierError::HttpError(e) => {
                if e.is_timeout() {
                    ClassifierErrorType::DomainFetchTimeoutError
                } else if e.is_connect() {
                    ClassifierErrorType::OllamaApiConnectionError
                } else if e.is_status() {
                    ClassifierErrorType::OllamaApiError
                } else {
                    ClassifierErrorType::DomainFetchError
                }
            }
            ClassifierError::JsonError(_) => {
                ClassifierErrorType::MetadataSerializationError
            }
        }
    }
}
