//! Model Evaluation Tests
//!
//! These tests are disabled by default (#[ignore]) and are meant to be run
//! manually to evaluate how different Ollama models perform with the
//! classification prompts.
//!
//! To run these tests:
//!     cargo test --test model_evaluation_test -- --ignored
//!
//! To run a specific model test:
//!     cargo test --test model_evaluation_test test_llama3_2_3b -- --ignored
//!
//! Requirements:
//! - Ollama server running (default: http://localhost:11434)
//! - Models downloaded (use `ollama pull <model>`)
//! - Set OLLAMA_URL environment variable if not using default

use dns_smart_block_classifier::{classify_with_llm, web_classify::SiteMetadata};
use std::env;

/// Get Ollama URL from environment or use default.
fn get_ollama_url() -> String {
    env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string())
}

/// Sample gaming classification prompt template.
const GAMING_PROMPT_TEMPLATE: &str = r#"
You are a strict JSON-only classifier.

Your task:
- Decide whether a given website is a "gaming site" under the following rules.
- Then output ONLY a JSON object with two fields:
  - "is_matching_site": a boolean
  - "confidence": a number between 0 and 1 (decimals allowed)

DEFINITION OF "GAMING SITE"

Gaming sites are those whose PRIMARY purpose is entertainment and recreation.
The key question is: Is this designed primarily for fun/entertainment, or
primarily for structured learning?

INCLUDE sites that are primarily games, even if they teach skills or have
educational value as a side effect (e.g., Minecraft, Kerbal Space Program,
Screeps, typing games where competition is the main draw).

EXCLUDE educational platforms where learning is the primary goal, even if they
use gamification for motivation (e.g., Duolingo, Babbel, Khan Academy,
Codecademy, educational tools designed by educators).

A site SHOULD be classified as a gaming site ("is_matching_site": true) if its
PRIMARY purpose is one or more of:

1. Being about games:
   - Video game news, reviews, previews, guides, walkthroughs.
   - Information or media focused mainly on games.

2. Being a platform for playing or distributing games:
   - Hosts playable games directly in the browser or via downloads.
   - Launchers, portals, or catalogs for playing or acquiring games.

A site SHOULD NOT be classified as a gaming site ("is_matching_site": false) if:

1. It is a general-purpose social network, chat platform, or content host
   whose main purpose is NOT specifically gaming.
   - Examples: reddit.com, facebook.com, discord.com, youtube.com

2. It is obviously about something else:
   - News, banking, education, general tech, streaming for all topics, etc.

INPUT FORMAT

You will receive a single JSON object describing the site. The JSON representation
will be provided below.

IMPORTANT OUTPUT RULES

- Output MUST be valid JSON.
- Output MUST have exactly these two fields at the top level:
  - "is_matching_site": boolean
  - "confidence": number between 0 and 1
- Do NOT include any extra text, comments, or explanations outside the JSON.
- Do NOT wrap the JSON in backticks.

NOW CLASSIFY THIS SITE

Input:
{{INPUT_JSON}}

Output:
"#;

/// Create test metadata for a known gaming site (Steam).
fn create_gaming_site_metadata() -> SiteMetadata {
    SiteMetadata {
        domain: "steampowered.com".to_string(),
        title: Some("Steam - The Ultimate Destination for Playing, Discussing, and Creating Games".to_string()),
        description: Some(
            "Steam is the ultimate destination for playing, discussing, and creating games."
                .to_string(),
        ),
        og_title: Some("Steam".to_string()),
        og_description: Some(
            "Steam is the ultimate online gaming platform. Download and play thousands of games."
                .to_string(),
        ),
        og_site_name: Some("Steam".to_string()),
        language: Some("en".to_string()),
        http_status: 200,
        fetch_error: None,
    }
}

/// Create test metadata for a non-gaming site (Wikipedia).
fn create_non_gaming_site_metadata() -> SiteMetadata {
    SiteMetadata {
        domain: "wikipedia.org".to_string(),
        title: Some("Wikipedia, the free encyclopedia".to_string()),
        description: Some(
            "Wikipedia is a free online encyclopedia with millions of articles on various topics."
                .to_string(),
        ),
        og_title: Some("Wikipedia".to_string()),
        og_description: Some(
            "The free encyclopedia that anyone can edit"
                .to_string(),
        ),
        og_site_name: Some("Wikipedia".to_string()),
        language: Some("en".to_string()),
        http_status: 200,
        fetch_error: None,
    }
}

/// Helper function to test any model with gaming classification.
///
/// Returns the classification result and prints timing information.
async fn test_model_classification(
    model: &str,
    metadata: &SiteMetadata,
    expected_is_matching: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let ollama_url = get_ollama_url();

    println!("\n=== Testing model: {} ===", model);
    println!("Ollama URL: {}", ollama_url);
    println!("Domain: {}", metadata.domain);
    println!("Expected is_matching_site: {}", expected_is_matching);

    let start = std::time::Instant::now();

    let result = classify_with_llm(
        metadata,
        &ollama_url,
        model,
        GAMING_PROMPT_TEMPLATE,
    )
    .await?;

    let duration = start.elapsed();

    println!("Result:");
    println!("  is_matching_site: {}", result.is_matching_site);
    println!("  confidence: {:.3}", result.confidence);
    println!("  duration: {:.2}s", duration.as_secs_f64());

    // Verify the result matches expectations.
    assert_eq!(
        result.is_matching_site, expected_is_matching,
        "Model {} gave unexpected classification for {}",
        model, metadata.domain
    );

    // Verify confidence is in valid range.
    assert!(
        result.confidence >= 0.0 && result.confidence <= 1.0,
        "Confidence out of range: {}",
        result.confidence
    );

    println!("✓ Test passed for model: {}", model);

    Ok(())
}

// ============================================================================
// Individual Model Tests
// ============================================================================
// Each test is marked with #[ignore] to be disabled by default.
// Run with: cargo test --test model_evaluation_test <test_name> -- --ignored

#[tokio::test]
#[ignore]
async fn test_llama3_2_3b_gaming_site() {
    test_model_classification(
        "llama3.2:3b",
        &create_gaming_site_metadata(),
        true,
    )
    .await
    .expect("llama3.2:3b test failed");
}

#[tokio::test]
#[ignore]
async fn test_llama3_2_3b_non_gaming_site() {
    test_model_classification(
        "llama3.2:3b",
        &create_non_gaming_site_metadata(),
        false,
    )
    .await
    .expect("llama3.2:3b test failed");
}

#[tokio::test]
#[ignore]
async fn test_llama3_1_8b_gaming_site() {
    test_model_classification(
        "llama3.1:8b",
        &create_gaming_site_metadata(),
        true,
    )
    .await
    .expect("llama3.1:8b test failed");
}

#[tokio::test]
#[ignore]
async fn test_llama3_1_8b_non_gaming_site() {
    test_model_classification(
        "llama3.1:8b",
        &create_non_gaming_site_metadata(),
        false,
    )
    .await
    .expect("llama3.1:8b test failed");
}

#[tokio::test]
#[ignore]
async fn test_qwen2_5_7b_instruct_gaming_site() {
    test_model_classification(
        "qwen2.5:7b-instruct",
        &create_gaming_site_metadata(),
        true,
    )
    .await
    .expect("qwen2.5:7b-instruct test failed");
}

#[tokio::test]
#[ignore]
async fn test_qwen2_5_7b_instruct_non_gaming_site() {
    test_model_classification(
        "qwen2.5:7b-instruct",
        &create_non_gaming_site_metadata(),
        false,
    )
    .await
    .expect("qwen2.5:7b-instruct test failed");
}

#[tokio::test]
#[ignore]
async fn test_mistral_7b_instruct_gaming_site() {
    test_model_classification(
        "mistral:7b-instruct",
        &create_gaming_site_metadata(),
        true,
    )
    .await
    .expect("mistral:7b-instruct test failed");
}

#[tokio::test]
#[ignore]
async fn test_mistral_7b_instruct_non_gaming_site() {
    test_model_classification(
        "mistral:7b-instruct",
        &create_non_gaming_site_metadata(),
        false,
    )
    .await
    .expect("mistral:7b-instruct test failed");
}

#[tokio::test]
#[ignore]
async fn test_phi3_mini_gaming_site() {
    test_model_classification(
        "phi3:mini",
        &create_gaming_site_metadata(),
        true,
    )
    .await
    .expect("phi3:mini test failed");
}

#[tokio::test]
#[ignore]
async fn test_phi3_mini_non_gaming_site() {
    test_model_classification(
        "phi3:mini",
        &create_non_gaming_site_metadata(),
        false,
    )
    .await
    .expect("phi3:mini test failed");
}

// ============================================================================
// Custom Model Test
// ============================================================================
// This test can be used to test any model by setting the MODEL environment
// variable.
//
// Example:
//   MODEL=llama3.2:1b cargo test --test model_evaluation_test test_custom_model -- --ignored

#[tokio::test]
#[ignore]
async fn test_custom_model_gaming_site() {
    let model = env::var("MODEL").expect(
        "MODEL environment variable must be set to use test_custom_model. \
         Example: MODEL=llama3.2:1b cargo test test_custom_model -- --ignored"
    );

    test_model_classification(
        &model,
        &create_gaming_site_metadata(),
        true,
    )
    .await
    .expect("Custom model test failed");
}

#[tokio::test]
#[ignore]
async fn test_custom_model_non_gaming_site() {
    let model = env::var("MODEL").expect(
        "MODEL environment variable must be set to use test_custom_model. \
         Example: MODEL=llama3.2:1b cargo test test_custom_model -- --ignored"
    );

    test_model_classification(
        &model,
        &create_non_gaming_site_metadata(),
        false,
    )
    .await
    .expect("Custom model test failed");
}

// ============================================================================
// Batch Test - Run all models sequentially
// ============================================================================
// This test runs all models in sequence and compares their performance.

#[tokio::test]
#[ignore]
async fn test_all_models_comparison() {
    let models = vec![
        "llama3.2:3b",
        "llama3.1:8b",
        "qwen2.5:7b-instruct",
        "mistral:7b-instruct",
        "phi3:mini",
    ];

    println!("\n{}", "=".repeat(70));
    println!("RUNNING COMPARISON TEST FOR ALL MODELS");
    println!("{}", "=".repeat(70));

    let gaming_metadata = create_gaming_site_metadata();
    let non_gaming_metadata = create_non_gaming_site_metadata();

    for model in models {
        println!("\n--- Testing {} ---", model);

        // Test gaming site.
        match test_model_classification(model, &gaming_metadata, true).await {
            Ok(_) => println!("✓ Gaming site test passed"),
            Err(e) => {
                eprintln!("✗ Gaming site test failed: {}", e);
                println!("  (Skipping non-gaming test for this model)");
                continue;
            }
        }

        // Test non-gaming site.
        match test_model_classification(model, &non_gaming_metadata, false).await {
            Ok(_) => println!("✓ Non-gaming site test passed"),
            Err(e) => {
                eprintln!("✗ Non-gaming site test failed: {}", e);
            }
        }
    }

    println!("\n{}", "=".repeat(70));
    println!("COMPARISON TEST COMPLETE");
    println!("{}\n", "=".repeat(70));
}
