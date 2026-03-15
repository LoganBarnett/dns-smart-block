use clap::Parser;
use dns_smart_block_classifier::{
    classify_with_llm, cli_args::CliArgs, compute_prompt_hash, error::ClassifierError,
    output::{
        ClassificationMetadata, ClassificationOutput, ErrorInfo, ErrorOutput, PartialMetadata,
    },
    web_classify::{extract_metadata, fetch_domain},
};
use tracing::{error, info};

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();

    // Initialize logging with auto-detection and CLI overrides
    args.logging.init_tracing();

    info!("========================================");
    info!("Starting DNS Smart Block Classifier");
    info!("========================================");
    info!("Configuration:");
    info!("  Domain: {}", args.domain);
    info!("  Classification Type: {}", args.classification_type);
    info!("  Ollama URL: {}", args.ollama_url);
    info!("  Ollama Model: {}", args.ollama_model);
    info!("  Prompt Template: {:?}", args.prompt_template);
    info!("  HTTP Timeout: {}s", args.http_timeout_sec);
    info!("  HTTP Max KB: {}", args.http_max_kb);
    info!("  Output Format: {}", args.output);
    info!("========================================");

    // Run classification and always produce output
    let result = run_classification(&args).await;

    // Output to stdout based on format
    match args.output.as_str() {
        "json" => {
            // JSON output - always produce valid JSON
            let json = match result {
                Ok(output) => output.to_json().unwrap_or_else(|e| {
                    error!("Failed to serialize output: {}", e);
                    format!(r#"{{"error": "Failed to serialize output"}}"#)
                }),
                Err(error_output) => error_output.to_json().unwrap_or_else(|e| {
                    error!("Failed to serialize error output: {}", e);
                    format!(r#"{{"error": "Failed to serialize error output"}}"#)
                }),
            };
            println!("{}", json);
        }
        _ => {
            // Human-readable output
            match result {
                Ok(output) => {
                    println!("Classification Result:");
                    println!("  Domain: {}", output.domain);
                    println!(
                        "  Is Matching Site: {}",
                        output.classification.is_matching_site
                    );
                    println!("  Confidence: {:.2}", output.classification.confidence);
                    println!("  HTTP Status: {}", output.metadata.http_status);
                    println!("  Model: {}", output.metadata.model);
                    println!("  Prompt Hash: {}", output.metadata.prompt_hash);
                }
                Err(error_output) => {
                    eprintln!("Classification Error:");
                    eprintln!("  Domain: {}", error_output.domain);
                    eprintln!("  Error Type: {}", error_output.error.error_type);
                    eprintln!("  Message: {}", error_output.error.message);
                    std::process::exit(1);
                }
            }
        }
    }
}

async fn run_classification(
    args: &CliArgs,
) -> Result<ClassificationOutput, ErrorOutput> {
    use std::time::Instant;
    let start_time = Instant::now();

    // Read prompt template
    info!("Step 1/3: Reading prompt template...");
    let prompt_template = std::fs::read_to_string(&args.prompt_template).map_err(|e| {
        error!(
            "Failed to read prompt template from {:?}: {}",
            args.prompt_template, e
        );
        let err = ClassifierError::from(e);
        ErrorOutput {
            domain: args.domain.clone(),
            result: "error".to_string(),
            error: ErrorInfo {
                error_type: err.to_error_type(),
                message: err.to_string(),
            },
            metadata: None,
        }
    })?;

    let prompt_hash = compute_prompt_hash(&prompt_template);
    info!("  Prompt template loaded (hash: {})", prompt_hash);
    info!("  Prompt length: {} characters", prompt_template.len());

    // Fetch domain content (best-effort - continue even if it fails)
    info!("Step 2/3: Fetching domain content from {}...", args.domain);
    let fetch_start = Instant::now();
    let metadata = match fetch_domain(&args.domain, args.http_timeout_sec, args.http_max_kb).await
    {
        Ok((html, status)) => {
            info!(
                "  HTTP fetch succeeded: status={}, size={} bytes, elapsed={:.2}s",
                status,
                html.len(),
                fetch_start.elapsed().as_secs_f64()
            );
            // Successfully fetched - extract metadata from HTML
            extract_metadata(&args.domain, &html, status).unwrap_or_else(|e| {
                error!("Failed to extract metadata from HTML: {}", e);
                // Fall back to minimal metadata with fetch error
                use dns_smart_block_classifier::web_classify::SiteMetadata;
                SiteMetadata::from_fetch_error(
                    &args.domain,
                    &format!("Metadata extraction failed: {}", e),
                )
            })
        }
        Err(e) => {
            error!(
                "  HTTP fetch failed after {:.2}s: {}",
                fetch_start.elapsed().as_secs_f64(),
                e
            );
            // HTTP fetch failed - create minimal metadata with just domain name
            use dns_smart_block_classifier::web_classify::SiteMetadata;
            SiteMetadata::from_fetch_error(&args.domain, &e.to_string())
        }
    };

    info!("  Extracted metadata:");
    info!("    Title: {:?}", metadata.title);
    info!("    Language: {:?}", metadata.language);
    info!("    HTTP Status: {}", metadata.http_status);

    // Classify with LLM
    info!("Step 3/3: Classifying with LLM...");
    info!("  Calling Ollama API at {}", args.ollama_url);
    info!("  Using model: {}", args.ollama_model);
    let llm_start = Instant::now();
    let classification = classify_with_llm(
        &metadata,
        &args.ollama_url,
        &args.ollama_model,
        &prompt_template,
    )
    .await
    .map_err(|e| {
        error!(
            "  LLM classification failed after {:.2}s: {}",
            llm_start.elapsed().as_secs_f64(),
            e
        );
        ErrorOutput {
            domain: args.domain.clone(),
            result: "error".to_string(),
            error: ErrorInfo {
                error_type: e.to_error_type(),
                message: e.to_string(),
            },
            metadata: Some(PartialMetadata {
                model: args.ollama_model.clone(),
                prompt_hash: prompt_hash.clone(),
            }),
        }
    })?;

    info!(
        "  LLM classification succeeded in {:.2}s",
        llm_start.elapsed().as_secs_f64()
    );
    info!(
        "Classification Result: is_matching={}, confidence={:.2}",
        classification.is_matching_site, classification.confidence
    );
    info!(
        "Total execution time: {:.2}s",
        start_time.elapsed().as_secs_f64()
    );

    Ok(ClassificationOutput {
        domain: args.domain.clone(),
        result: "classified".to_string(),
        classification,
        metadata: ClassificationMetadata {
            http_status: metadata.http_status,
            model: args.ollama_model.clone(),
            prompt_hash,
        },
    })
}
