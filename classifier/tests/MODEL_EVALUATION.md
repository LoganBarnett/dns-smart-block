# Model Evaluation Tests

This directory contains integration tests for evaluating different Ollama models
with the DNS Smart Block classification prompts.

## Overview

The model evaluation tests (`model_evaluation_test.rs`) are disabled by default
(marked with `#[ignore]`) and are meant to be run manually to:

- Compare how different models perform with the gaming classification prompt
- Measure response time for each model
- Verify that models can correctly classify gaming vs. non-gaming sites
- Test custom or experimental models

## Prerequisites

1. **Ollama server running**: The tests connect to Ollama (default:
   `http://localhost:11434`)
2. **Models downloaded**: Use `ollama pull <model>` to download models before
   testing
3. **Environment variables** (optional):
   - `OLLAMA_URL`: Override the default Ollama server URL
   - `MODEL`: Specify a custom model for the `test_custom_model_*` tests

## Running Tests

### Run All Model Evaluation Tests

```bash
cd /Users/logan/dev/dns-smart-block/classifier
cargo test --test model_evaluation_test -- --ignored
```

### Run a Specific Model Test

```bash
# Test llama3.2:3b with a gaming site
cargo test --test model_evaluation_test test_llama3_2_3b_gaming_site -- --ignored

# Test qwen2.5:7b-instruct with a non-gaming site
cargo test --test model_evaluation_test test_qwen2_5_7b_instruct_non_gaming_site -- --ignored
```

### Run the Comparison Test (All Models)

This runs all configured models sequentially and reports their performance:

```bash
cargo test --test model_evaluation_test test_all_models_comparison -- --ignored
```

### Test a Custom Model

Use the `MODEL` environment variable to test any model:

```bash
MODEL=llama3.2:1b cargo test --test model_evaluation_test test_custom_model -- --ignored
```

### Use a Different Ollama Server

```bash
OLLAMA_URL=http://remote-server:11434 cargo test --test model_evaluation_test test_llama3_2_3b_gaming_site -- --ignored
```

## Available Model Tests

The following models have pre-configured tests:

- **llama3.2:3b** - Small, fast Llama 3.2 model (instruct-tuned)
- **llama3.1:8b** - Larger Llama 3.1 model (instruct-tuned)
- **qwen2.5:7b-instruct** - Qwen 2.5 7B instruct model
- **mistral:7b-instruct** - Mistral 7B instruct model
- **phi3:mini** - Microsoft Phi-3 mini model

Each model has two tests:
- `test_<model>_gaming_site` - Tests classification of a gaming site (Steam)
- `test_<model>_non_gaming_site` - Tests classification of a non-gaming site
  (Wikipedia)

## Understanding Test Output

When you run a test, you'll see output like:

```
=== Testing model: llama3.2:3b ===
Ollama URL: http://localhost:11434
Domain: steampowered.com
Expected is_matching_site: true
Result:
  is_matching_site: true
  confidence: 0.950
  duration: 4.23s
✓ Test passed for model: llama3.2:3b
```

Key metrics:
- **is_matching_site**: Whether the model classified the site correctly
- **confidence**: The model's confidence score (0.0 to 1.0)
- **duration**: How long the classification took (includes model load time)

## Adding New Model Tests

To add tests for a new model:

1. Add two new test functions in `model_evaluation_test.rs`:

```rust
#[tokio::test]
#[ignore]
async fn test_new_model_gaming_site() {
    test_model_classification(
        "new-model:tag",
        &create_gaming_site_metadata(),
        true,
    )
    .await
    .expect("new-model test failed");
}

#[tokio::test]
#[ignore]
async fn test_new_model_non_gaming_site() {
    test_model_classification(
        "new-model:tag",
        &create_non_gaming_site_metadata(),
        false,
    )
    .await
    .expect("new-model test failed");
}
```

2. Add the model to the `test_all_models_comparison` test's model list.

## Test Data

The tests use two predefined site metadata examples:

1. **Gaming site**: steampowered.com (Steam)
   - Should be classified as `is_matching_site: true`
   - Typical confidence: > 0.9

2. **Non-gaming site**: wikipedia.org (Wikipedia)
   - Should be classified as `is_matching_site: false`
   - Typical confidence: > 0.9

## Troubleshooting

### Test hangs or times out

- The model may be too large and taking too long to load
- Check if Ollama is running: `curl http://localhost:11434/api/tags`
- Try a smaller model first (like `llama3.2:3b`)

### Model not found error

Download the model first:

```bash
ollama pull llama3.2:3b
```

### Connection refused

- Ensure Ollama is running: `ollama serve`
- Check the OLLAMA_URL is correct

### Unexpected classification results

- Some models may perform better or worse than others
- Try adjusting the prompt template if needed
- Check the confidence score - low confidence may indicate the model is uncertain

## Performance Comparison

Typical performance on a MacBook Pro (M1):

| Model | Size | Load Time | Inference Time | Total Time |
|-------|------|-----------|----------------|------------|
| llama3.2:3b | 2GB | ~12s | ~5s | ~17s |
| llama3.1:8b | 4.7GB | ~18s | ~8s | ~26s |
| qwen2.5:7b-instruct | 4.7GB | ~20s | ~12s | ~32s |
| phi3:mini | 2.3GB | ~10s | ~6s | ~16s |

*Note: First load times are longer. Subsequent classifications with the same
model are faster if the model stays in memory.*

## Integration with Production

These tests help you:

1. **Choose the right model** for your deployment based on:
   - Classification accuracy
   - Response time
   - Resource requirements

2. **Validate prompt changes** by running the comparison test before and after
   modifying prompts

3. **Test new models** before deploying them to production

## See Also

- `/Users/logan/dev/dns-smart-block/prompts/` - Prompt templates used in
  production
- `/Users/logan/dev/dns-smart-block/classifier/tests/integration_test.rs` -
  Mock-based integration tests
- [Ollama Model Library](https://ollama.com/library) - Available models
