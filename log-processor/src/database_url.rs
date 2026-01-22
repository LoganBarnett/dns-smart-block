use std::fs;
use std::path::Path;
use thiserror::Error;
use url::Url;

#[derive(Error, Debug)]
pub enum DatabaseUrlError {
    #[error("Failed to read password file: {0}")]
    PasswordFileError(#[from] std::io::Error),

    #[error("Failed to parse database URL: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Database URL is missing")]
    MissingUrl,
}

/// Construct a database URL with password from file if provided
pub fn construct_database_url(
    base_url: &str,
    password_file: Option<&Path>,
) -> Result<String, DatabaseUrlError> {
    if let Some(password_path) = password_file {
        // Read password from file
        let password = fs::read_to_string(password_path)?
            .trim()
            .to_string();

        // Parse the base URL and inject password
        let mut url = Url::parse(base_url)?;
        url.set_password(Some(&password))
            .map_err(|_| DatabaseUrlError::UrlParseError(url::ParseError::InvalidDomainCharacter))?;

        Ok(url.to_string())
    } else {
        // No password file, use URL as-is
        Ok(base_url.to_string())
    }
}

/// Sanitize a database URL for logging (hide password)
pub fn sanitize_database_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut parsed) => {
            if parsed.password().is_some() {
                // Replace password with asterisks
                let _ = parsed.set_password(Some("***"));
            }
            parsed.to_string()
        }
        Err(_) => {
            // If parsing fails, try to redact password manually
            // This handles cases like postgresql://user:pass@/db?host=/socket
            if url.contains(':') && url.contains('@') {
                // Find password between : and @
                if let Some(start) = url.find("://") {
                    if let Some(colon) = url[start+3..].find(':') {
                        if let Some(at) = url[start+3+colon..].find('@') {
                            let before = &url[..start+3+colon+1];
                            let after = &url[start+3+colon+at+1..];
                            return format!("{}***{}", before, after);
                        }
                    }
                }
            }
            // If no password or can't parse, return as-is
            url.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_sanitize_url_with_password() {
        let url = "postgresql://user:secret@localhost/db";
        let sanitized = sanitize_database_url(url);
        assert!(!sanitized.contains("secret"));
        assert!(sanitized.contains("***"));
        assert!(sanitized.contains("user"));
        assert!(sanitized.contains("localhost"));
    }

    #[test]
    fn test_sanitize_url_without_password() {
        let url = "postgresql://user@localhost/db";
        let sanitized = sanitize_database_url(url);
        assert_eq!(sanitized, url);
    }

    #[test]
    fn test_sanitize_unix_socket_url() {
        let url = "postgresql://user@/db?host=/run/postgresql";
        let sanitized = sanitize_database_url(url);
        assert_eq!(sanitized, url);
    }

    #[test]
    fn test_construct_url_without_password_file() {
        let url = "postgresql://user@localhost/db";
        let result = construct_database_url(url, None).unwrap();
        assert_eq!(result, url);
    }

    #[test]
    fn test_construct_url_with_password_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "my_secret_password").unwrap();

        let base_url = "postgresql://user@localhost/db";
        let result = construct_database_url(base_url, Some(temp_file.path())).unwrap();

        assert!(result.contains("my_secret_password"));
        assert!(result.contains("user"));
        assert!(result.contains("localhost"));

        // Verify it can be parsed
        let parsed = Url::parse(&result).unwrap();
        assert_eq!(parsed.password(), Some("my_secret_password"));
    }

    #[test]
    fn test_construct_url_trims_password() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "  my_password  ").unwrap();

        let base_url = "postgresql://user@localhost/db";
        let result = construct_database_url(base_url, Some(temp_file.path())).unwrap();

        let parsed = Url::parse(&result).unwrap();
        assert_eq!(parsed.password(), Some("my_password"));
    }
}
