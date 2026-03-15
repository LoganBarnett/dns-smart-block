// These tests verify the embedded template and static assets that are compiled
// into the binary via include_str!/include_bytes!.  The compiler enforces that
// the source files exist at build time, so there is no need to test for file
// presence at runtime.

// Re-export the constants from main so we can test them here.  These are
// embedded at compile time and will fail the build if the source files are
// missing or cannot be read.
use dns_smart_block_blocklist_server::CLASSIFICATIONS_CSS;
use dns_smart_block_blocklist_server::CLASSIFICATIONS_HTML;
use dns_smart_block_blocklist_server::CLASSIFICATIONS_JS;

#[test]
fn test_template_contains_placeholders() {
  assert!(
    CLASSIFICATIONS_HTML.contains("{{FILTER_INFO}}"),
    "Template should contain {{{{FILTER_INFO}}}} placeholder."
  );
  assert!(
    CLASSIFICATIONS_HTML.contains("{{COUNT}}"),
    "Template should contain {{{{COUNT}}}} placeholder."
  );
  assert!(
    CLASSIFICATIONS_HTML.contains("{{ROWS}}"),
    "Template should contain {{{{ROWS}}}} placeholder."
  );
}

#[test]
fn test_template_references_static_assets() {
  assert!(
    CLASSIFICATIONS_HTML.contains("/static/classifications.css"),
    "Template should reference the embedded CSS asset."
  );
  assert!(
    CLASSIFICATIONS_HTML.contains("/static/classifications.js"),
    "Template should reference the embedded JS asset."
  );
}

#[test]
fn test_static_assets_non_empty() {
  assert!(
    !CLASSIFICATIONS_CSS.is_empty(),
    "Embedded CSS should not be empty."
  );
  assert!(
    !CLASSIFICATIONS_JS.is_empty(),
    "Embedded JS should not be empty."
  );
}

#[test]
fn test_static_assets_content() {
  let css = std::str::from_utf8(CLASSIFICATIONS_CSS)
    .expect("CSS should be valid UTF-8.");
  let js =
    std::str::from_utf8(CLASSIFICATIONS_JS).expect("JS should be valid UTF-8.");

  assert!(css.contains("body"), "CSS should contain body styles.");
  assert!(
    js.contains("sortTable"),
    "JS should contain sortTable function."
  );
  assert!(
    js.contains("expireDomain"),
    "JS should contain expireDomain function."
  );
}

#[test]
fn test_template_substitution() {
  let result = CLASSIFICATIONS_HTML
    .replace("{{FILTER_INFO}}", " - Test Filter")
    .replace("{{COUNT}}", "42")
    .replace("{{ROWS}}", "<tr><td>test</td></tr>");

  assert!(
    result.contains("Total: 42 classification(s)"),
    "Template should render count correctly."
  );
  assert!(
    result.contains("<tr><td>test</td></tr>"),
    "Template should render rows correctly."
  );
}
