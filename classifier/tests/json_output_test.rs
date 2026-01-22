use dns_smart_block_classifier::output::{
    Classification, ClassificationMetadata, ClassificationOutput, ErrorInfo, ErrorOutput,
};
use dns_smart_block_classifier::error::ClassifierErrorType;
use dns_smart_block_classifier::compute_prompt_hash;
use serde_json;

#[test]
fn test_classification_output_serialization() {
    let output = ClassificationOutput {
        domain: "gaming-site.com".to_string(),
        result: "classified".to_string(),
        classification: Classification {
            is_matching_site: true,
            confidence: 0.95,
        },
        metadata: ClassificationMetadata {
            model: "llama2".to_string(),
            prompt_hash: "sha256:abcd1234".to_string(),
            http_status: 200,
        },
    };

    let json = output.to_json().expect("Failed to serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("Invalid JSON");

    assert_eq!(parsed["domain"], "gaming-site.com");
    assert_eq!(parsed["result"], "classified");
    assert_eq!(parsed["classification"]["is_matching_site"], true);
    assert_eq!(parsed["classification"]["confidence"], 0.95);
    assert_eq!(parsed["metadata"]["model"], "llama2");
    assert_eq!(parsed["metadata"]["prompt_hash"], "sha256:abcd1234");
    assert_eq!(parsed["metadata"]["http_status"], 200);
}

#[test]
fn test_error_output_serialization() {
    let output = ErrorOutput {
        domain: "error-domain.com".to_string(),
        result: "error".to_string(),
        error: ErrorInfo {
            error_type: ClassifierErrorType::OllamaApiTimeoutError,
            message: "Connection timed out after 30s".to_string(),
        },
        metadata: None,
    };

    let json = output.to_json().expect("Failed to serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("Invalid JSON");

    assert_eq!(parsed["domain"], "error-domain.com");
    assert_eq!(parsed["result"], "error");
    assert_eq!(parsed["error"]["error_type"], "OllamaApiTimeoutError");
    assert_eq!(parsed["error"]["message"], "Connection timed out after 30s");
}

#[test]
fn test_error_output_with_partial_metadata() {
    let output = ErrorOutput {
        domain: "partial-error.com".to_string(),
        result: "error".to_string(),
        error: ErrorInfo {
            error_type: ClassifierErrorType::DomainFetchTimeoutError,
            message: "Failed to fetch domain content".to_string(),
        },
        metadata: Some(dns_smart_block_classifier::output::PartialMetadata {
            model: "llama2".to_string(),
            prompt_hash: "sha256:xyz789".to_string(),
        }),
    };

    let json = output.to_json().expect("Failed to serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("Invalid JSON");

    assert_eq!(parsed["domain"], "partial-error.com");
    assert_eq!(parsed["result"], "error");
    assert_eq!(parsed["error"]["error_type"], "DomainFetchTimeoutError");
    assert_eq!(parsed["metadata"]["model"], "llama2");
    assert_eq!(parsed["metadata"]["prompt_hash"], "sha256:xyz789");
}

#[test]
fn test_classification_output_deserialization() {
    let json = r#"{
        "domain": "test-site.com",
        "result": "classified",
        "classification": {
            "is_matching_site": false,
            "confidence": 0.12
        },
        "metadata": {
            "model": "llama3",
            "prompt_hash": "sha256:test123",
            "http_status": 200
        }
    }"#;

    let output: ClassificationOutput = serde_json::from_str(json).expect("Failed to deserialize");

    assert_eq!(output.domain, "test-site.com");
    assert_eq!(output.result, "classified");
    assert_eq!(output.classification.is_matching_site, false);
    assert!((output.classification.confidence - 0.12).abs() < 0.001);
    assert_eq!(output.metadata.model, "llama3");
    assert_eq!(output.metadata.prompt_hash, "sha256:test123");
    assert_eq!(output.metadata.http_status, 200);
}

#[test]
fn test_error_output_deserialization() {
    let json = r#"{
        "domain": "error-site.com",
        "result": "error",
        "error": {
            "error_type": "OllamaApiError",
            "message": "Model not found"
        }
    }"#;

    let output: ErrorOutput = serde_json::from_str(json).expect("Failed to deserialize");

    assert_eq!(output.domain, "error-site.com");
    assert_eq!(output.result, "error");
    assert_eq!(output.error.error_type, ClassifierErrorType::OllamaApiError);
    assert_eq!(output.error.message, "Model not found");
    assert!(output.metadata.is_none());
}

#[test]
fn test_all_error_types_serialize() {
    let error_types = vec![
        ClassifierErrorType::PromptFileReadError,
        ClassifierErrorType::DomainFetchError,
        ClassifierErrorType::DomainFetchTimeoutError,
        ClassifierErrorType::HtmlParseError,
        ClassifierErrorType::OllamaApiConnectionError,
        ClassifierErrorType::OllamaApiTimeoutError,
        ClassifierErrorType::OllamaApiError,
        ClassifierErrorType::OllamaResponseParseError,
        ClassifierErrorType::ClassificationParseError,
        ClassifierErrorType::MetadataSerializationError,
    ];

    for error_type in error_types {
        let output = ErrorOutput {
            domain: "test.com".to_string(),
            result: "error".to_string(),
            error: ErrorInfo {
                error_type: error_type.clone(),
                message: "test error".to_string(),
            },
            metadata: None,
        };

        let json = output.to_json().expect("Failed to serialize");
        let parsed: ErrorOutput = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(parsed.error.error_type, error_type);
    }
}

#[test]
fn test_compute_prompt_hash() {
    let prompt1 = "This is a test prompt";
    let prompt2 = "This is a test prompt";
    let prompt3 = "This is a different prompt";

    let hash1 = compute_prompt_hash(prompt1);
    let hash2 = compute_prompt_hash(prompt2);
    let hash3 = compute_prompt_hash(prompt3);

    // Same content should produce same hash
    assert_eq!(hash1, hash2);

    // Different content should produce different hash
    assert_ne!(hash1, hash3);

    // Hash should have sha256: prefix
    assert!(hash1.starts_with("sha256:"));

    // Hash should be hex encoded (64 hex chars after prefix)
    let hex_part = hash1.strip_prefix("sha256:").unwrap();
    assert_eq!(hex_part.len(), 64);
    assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_confidence_values() {
    // Test various confidence values
    let test_cases = vec![0.0, 0.25, 0.5, 0.75, 0.8, 0.9, 0.95, 0.99, 1.0];

    for confidence in test_cases {
        let output = ClassificationOutput {
            domain: "test.com".to_string(),
            result: "classified".to_string(),
            classification: Classification {
                is_matching_site: true,
                confidence,
            },
            metadata: ClassificationMetadata {
                model: "test".to_string(),
                prompt_hash: "sha256:test".to_string(),
                http_status: 200,
            },
        };

        let json = output.to_json().expect("Failed to serialize");
        let parsed: ClassificationOutput = serde_json::from_str(&json).expect("Failed to deserialize");

        assert!((parsed.classification.confidence - confidence).abs() < 0.0001);
    }
}

#[test]
fn test_http_status_codes() {
    let status_codes = vec![200, 301, 302, 400, 403, 404, 500, 503];

    for status in status_codes {
        let output = ClassificationOutput {
            domain: "test.com".to_string(),
            result: "classified".to_string(),
            classification: Classification {
                is_matching_site: false,
                confidence: 0.5,
            },
            metadata: ClassificationMetadata {
                model: "test".to_string(),
                prompt_hash: "sha256:test".to_string(),
                http_status: status,
            },
        };

        let json = output.to_json().expect("Failed to serialize");
        let parsed: ClassificationOutput = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(parsed.metadata.http_status, status);
    }
}
