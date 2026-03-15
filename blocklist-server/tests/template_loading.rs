// These tests verify the embedded template and static assets that are compiled
// into the binary via include_str!/include_bytes!.  The compiler enforces that
// the source files exist at build time, so there is no need to test for file
// presence at runtime.

// Re-export the constants from main so we can test them here.  These are
// embedded at compile time and will fail the build if the source files are
// missing or cannot be read.
use dns_smart_block_blocklist_server::CLASSIFICATIONS_CSS;
use dns_smart_block_blocklist_server::CLASSIFICATIONS_HTML;
use dns_smart_block_blocklist_server::ELM_JS;

#[test]
fn test_template_references_static_assets() {
  assert!(
    CLASSIFICATIONS_HTML.contains("/static/classifications.css"),
    "Template should reference the embedded CSS asset."
  );
  assert!(
    CLASSIFICATIONS_HTML.contains("/static/elm.js"),
    "Template should reference the Elm JS asset."
  );
}

#[test]
fn test_template_mounts_elm() {
  assert!(
    CLASSIFICATIONS_HTML.contains("Elm.Main.init"),
    "Template should initialize the Elm app."
  );
  assert!(
    CLASSIFICATIONS_HTML.contains("id=\"app\""),
    "Template should contain the Elm mount point."
  );
}

#[test]
fn test_static_assets_non_empty() {
  assert!(
    !CLASSIFICATIONS_CSS.is_empty(),
    "Embedded CSS should not be empty."
  );
  assert!(!ELM_JS.is_empty(), "Embedded Elm JS should not be empty.");
}

#[test]
fn test_static_assets_content() {
  let css = std::str::from_utf8(CLASSIFICATIONS_CSS)
    .expect("CSS should be valid UTF-8.");
  let js = std::str::from_utf8(ELM_JS).expect("Elm JS should be valid UTF-8.");

  assert!(css.contains("body"), "CSS should contain body styles.");
  assert!(js.contains("Elm"), "Elm JS should contain the Elm runtime.");
}
