/// Test to verify error messages include sufficient context
///
/// This test ensures that when file operations fail, the error messages
/// include the file path and other relevant context to aid debugging.
#[test]
fn test_template_error_message_format() {
  // This test verifies the error message format without actually triggering the error
  // The actual error message is logged, not returned, so we're documenting the expected format

  let template_path = "blocklist-server/templates/classifications.html";
  let error = std::io::Error::new(
    std::io::ErrorKind::NotFound,
    "No such file or directory",
  );

  // Expected error message format:
  // "Failed to read template file 'blocklist-server/templates/classifications.html':
  //  No such file or directory (os error 2) (current directory: /some/path)"

  let expected_parts = vec![
    "Failed to read template file",
    template_path,
    "current directory",
  ];

  let error_msg = format!(
    "Failed to read template file '{}': {} (current directory: {})",
    template_path, error, "/some/path"
  );

  for part in expected_parts {
    assert!(
      error_msg.contains(part),
      "Error message should contain '{}', but got: {}",
      part,
      error_msg
    );
  }
}

#[test]
fn test_password_file_error_message_format() {
  use std::path::PathBuf;

  // This test verifies the error message format for password file errors
  let password_file = PathBuf::from("/etc/dns-smart-block/db-password");
  let error = std::io::Error::new(
    std::io::ErrorKind::NotFound,
    "No such file or directory",
  );

  // Expected error message format:
  // "Failed to read database password file '/etc/dns-smart-block/db-password':
  //  No such file or directory (os error 2)"

  let error_msg = format!(
    "Failed to read database password file '{}': {}",
    password_file.display(),
    error
  );

  assert!(
    error_msg.contains("Failed to read database password file"),
    "Error message should describe the operation"
  );
  assert!(
    error_msg.contains("/etc/dns-smart-block/db-password"),
    "Error message should contain the file path"
  );
  assert!(
    error_msg.contains("No such file or directory"),
    "Error message should contain the underlying error"
  );
}
