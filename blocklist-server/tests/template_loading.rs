use std::path::PathBuf;

/// Helper to find the project root by looking for Cargo.toml
fn find_project_root() -> PathBuf {
  let mut current =
    std::env::current_dir().expect("Failed to get current directory");

  // Try to find workspace root (contains top-level Cargo.toml)
  loop {
    let cargo_toml = current.join("Cargo.toml");
    if cargo_toml.exists() {
      // Check if this is the workspace root by looking for blocklist-server directory
      if current.join("blocklist-server").exists() {
        return current;
      }
    }

    if !current.pop() {
      break;
    }
  }

  // Fallback to current dir
  std::env::current_dir().expect("Failed to get current directory")
}

fn get_template_path() -> PathBuf {
  find_project_root().join("blocklist-server/templates/classifications.html")
}

fn get_css_path() -> PathBuf {
  find_project_root().join("blocklist-server/static/classifications.css")
}

fn get_js_path() -> PathBuf {
  find_project_root().join("blocklist-server/static/classifications.js")
}

#[test]
fn test_template_file_exists() {
  let template_path = get_template_path();
  assert!(
    template_path.exists(),
    "Template file should exist at {}",
    template_path.display()
  );
}

#[test]
fn test_template_file_readable() {
  let template_path = get_template_path();
  let content = std::fs::read_to_string(&template_path)
    .expect("Should be able to read template file");

  // Verify template contains expected placeholders
  assert!(
    content.contains("{{FILTER_INFO}}"),
    "Template should contain {{{{FILTER_INFO}}}} placeholder"
  );
  assert!(
    content.contains("{{COUNT}}"),
    "Template should contain {{{{COUNT}}}} placeholder"
  );
  assert!(
    content.contains("{{ROWS}}"),
    "Template should contain {{{{ROWS}}}} placeholder"
  );

  // Verify template references static assets
  assert!(
    content.contains("/static/classifications.css"),
    "Template should reference CSS file"
  );
  assert!(
    content.contains("/static/classifications.js"),
    "Template should reference JS file"
  );
}

#[test]
fn test_static_files_exist() {
  let css_path = get_css_path();
  let js_path = get_js_path();

  assert!(
    css_path.exists(),
    "CSS file should exist at {}",
    css_path.display()
  );
  assert!(
    js_path.exists(),
    "JS file should exist at {}",
    js_path.display()
  );
}

#[test]
fn test_static_files_readable() {
  let css_path = get_css_path();
  let js_path = get_js_path();

  let css_content = std::fs::read_to_string(&css_path)
    .expect("Should be able to read CSS file");
  let js_content =
    std::fs::read_to_string(&js_path).expect("Should be able to read JS file");

  // Basic sanity checks
  assert!(
    css_content.contains("body"),
    "CSS should contain body styles"
  );
  assert!(
    js_content.contains("sortTable"),
    "JS should contain sortTable function"
  );
  assert!(
    js_content.contains("expireDomain"),
    "JS should contain expireDomain function"
  );
}

#[test]
fn test_template_substitution() {
  let template_path = get_template_path();
  let template = std::fs::read_to_string(&template_path)
    .expect("Should be able to read template file");

  // Test basic substitution
  let result = template
    .replace("{{FILTER_INFO}}", " - Test Filter")
    .replace("{{COUNT}}", "42")
    .replace("{{ROWS}}", "<tr><td>test</td></tr>");

  // Verify substitution worked and no error message appears
  assert!(
    !result.contains("Error loading template"),
    "Rendered template should not contain error message"
  );
  assert!(
    result.contains("Total: 42 classification(s)"),
    "Template should render count correctly"
  );
  assert!(
    result.contains("<tr><td>test</td></tr>"),
    "Template should render rows correctly"
  );
}
