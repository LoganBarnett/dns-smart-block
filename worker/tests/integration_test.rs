use dns_smart_block_worker::{
    classify_with_llm, web_classify::SiteMetadata, ClassificationResult, OllamaResponse,
};
use serde_json::json;
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

/// Sample HTML content for a gaming site (Steam-like)
const GAMING_SITE_HTML: &str = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Awesome Game Store - Buy and Download PC Games</title>
    <meta name="description" content="The ultimate destination for PC gaming. Browse thousands of games, read reviews, and join our gaming community.">
    <meta property="og:title" content="Awesome Game Store">
    <meta property="og:description" content="Digital distribution platform for PC gaming with thousands of games available">
    <meta property="og:site_name" content="Awesome Game Store">
</head>
<body>
    <h1>Welcome to Awesome Game Store</h1>
    <p>Discover amazing games for PC, Mac, and Linux.</p>
</body>
</html>
"#;

/// Sample prompt template for gaming classification
const GAMING_PROMPT_TEMPLATE: &str = r#"
You are classifying websites to determine if they are gaming-related sites.

Analyze the following website metadata and determine if this is a gaming site
(game stores, game streaming platforms, game publishers, gaming communities, etc.):

{{INPUT_JSON}}

Respond ONLY with valid JSON in this exact format:
{
  "is_matching_site": true or false,
  "confidence": 0.0 to 1.0
}
"#;

/// Helper function to create a SiteMetadata for a gaming site
fn create_gaming_site_metadata() -> SiteMetadata {
    SiteMetadata {
        domain: "awesomegames.example".to_string(),
        title: Some("Awesome Game Store - Buy and Download PC Games".to_string()),
        description: Some(
            "The ultimate destination for PC gaming. Browse thousands of games, read reviews, and join our gaming community."
                .to_string(),
        ),
        og_title: Some("Awesome Game Store".to_string()),
        og_description: Some(
            "Digital distribution platform for PC gaming with thousands of games available"
                .to_string(),
        ),
        og_site_name: Some("Awesome Game Store".to_string()),
        language: Some("en".to_string()),
        http_status: 200,
    }
}

#[tokio::test]
async fn test_classify_gaming_site_with_mock_ollama() {
    // Set up the mock Ollama server
    let mock_server = MockServer::start().await;

    // Create the expected classification result
    let expected_result = ClassificationResult {
        is_matching_site: true,
        confidence: 0.95,
    };

    // Set up the mock response for the Ollama API
    let ollama_response = OllamaResponse {
        response: json!({
            "is_matching_site": true,
            "confidence": 0.95
        })
        .to_string(),
    };

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .and(body_partial_json(json!({
            "format": "json",
            "stream": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(&ollama_response))
        .mount(&mock_server)
        .await;

    // Create test metadata for a gaming site
    let metadata = create_gaming_site_metadata();

    // Call the classification function
    let result = classify_with_llm(
        &metadata,
        &mock_server.uri(),
        "test-model",
        GAMING_PROMPT_TEMPLATE,
    )
    .await
    .expect("Classification should succeed");

    // Assert the result
    assert_eq!(result, expected_result);
    assert!(result.is_matching_site);
    assert!(result.confidence > 0.9);
}

#[tokio::test]
async fn test_classify_non_gaming_site_with_mock_ollama() {
    // Set up the mock Ollama server
    let mock_server = MockServer::start().await;

    // Create a non-gaming site metadata
    let metadata = SiteMetadata {
        domain: "newssite.example".to_string(),
        title: Some("Daily News - Breaking News and Headlines".to_string()),
        description: Some("Get the latest news and breaking headlines from around the world.".to_string()),
        og_title: Some("Daily News".to_string()),
        og_description: Some("Your trusted source for news and current events".to_string()),
        og_site_name: Some("Daily News".to_string()),
        language: Some("en".to_string()),
        http_status: 200,
    };

    // Set up the mock response for a non-gaming site
    let ollama_response = OllamaResponse {
        response: json!({
            "is_matching_site": false,
            "confidence": 0.98
        })
        .to_string(),
    };

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&ollama_response))
        .mount(&mock_server)
        .await;

    // Call the classification function
    let result = classify_with_llm(
        &metadata,
        &mock_server.uri(),
        "test-model",
        GAMING_PROMPT_TEMPLATE,
    )
    .await
    .expect("Classification should succeed");

    // Assert the result
    assert!(!result.is_matching_site);
    assert!(result.confidence > 0.9);
}

#[tokio::test]
async fn test_ollama_api_error_handling() {
    // Set up the mock Ollama server that returns an error
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let metadata = create_gaming_site_metadata();

    // Call the classification function and expect an error
    let result = classify_with_llm(
        &metadata,
        &mock_server.uri(),
        "test-model",
        GAMING_PROMPT_TEMPLATE,
    )
    .await;

    assert!(result.is_err());
}

#[test]
fn test_gaming_site_html_parsing() {
    use dns_smart_block_worker::web_classify::extract_metadata;

    let metadata = extract_metadata("awesomegames.example", GAMING_SITE_HTML, 200)
        .expect("Should extract metadata");

    assert_eq!(metadata.domain, "awesomegames.example");
    assert!(metadata.title.is_some());
    assert!(metadata
        .title
        .as_ref()
        .unwrap()
        .contains("Awesome Game Store"));
    assert!(metadata.description.is_some());
    assert_eq!(metadata.http_status, 200);
}
