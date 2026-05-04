//! Host prefix wildcard pattern validation.
//!
//! Ensures that dotted host aliases have a corresponding wildcard pattern
//! (`Host prefix.*`) in the config files.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::check_types::{CheckLevel, CheckMessage};

/// Check that a host alias with multiple dot-separated parts has a matching
/// wildcard pattern in the config files.
///
/// For example, `org.env.host` should have a `Host org.env.*` or `Host org.*`
/// or `Host *` pattern defined somewhere.
///
/// # Errors
/// Returns an error if config files cannot be read.
pub fn check(host: &str, config_files: &[PathBuf]) -> Result<Vec<CheckMessage>> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() < 2 {
        return Ok(Vec::new());
    }

    // Build prefix candidates from most specific to least specific.
    // e.g. "org.env.host" -> ["org.env", "org"]
    let mut prefixes = Vec::new();
    for i in (1..parts.len()).rev() {
        let prefix = parts.get(..i).map(|p| p.join(".")).unwrap_or_default();
        prefixes.push(format!("{prefix}.*"));
    }

    // Read all config files once
    let mut config_content = String::new();
    for path in config_files {
        let content = std::fs::read_to_string(path).with_context(|| {
            format!("host_prefix: failed to read config file {}", path.display())
        })?;
        config_content.push_str(&content);
        config_content.push('\n');
    }

    // Check for matching Host pattern
    for prefix_pattern in &prefixes {
        if has_host_pattern(&config_content, prefix_pattern) {
            debug!(host = %host, pattern = %prefix_pattern, "found matching wildcard pattern");
            return Ok(Vec::new());
        }
    }

    // Check for `Host *` as fallback
    if has_host_pattern(&config_content, "*") {
        debug!(host = %host, "matched Host * fallback pattern");
        return Ok(Vec::new());
    }

    let default_pattern = String::from("*");
    let expected = prefixes.first().unwrap_or(&default_pattern);
    Ok(vec![CheckMessage {
        level: CheckLevel::Error,
        text: format!(
            "[{host}] No wildcard Host pattern found for prefix (expected e.g. Host {expected})"
        ),
    }])
}

/// Check if a `Host <pattern>` line exists in the config content.
fn has_host_pattern(content: &str, pattern: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        // Match "Host <pattern>" case-insensitively, where pattern may appear
        // among multiple space-separated patterns on the same Host line.
        if trimmed
            .strip_prefix("Host ")
            .or_else(|| trimmed.strip_prefix("host "))
            .is_some_and(|rest| rest.split_whitespace().any(|p| p == pattern))
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use std::io::Write as _;

    use super::*;

    fn write_config(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn single_part_host_skips() {
        // Arrange / Act
        let msgs = check("myhost", &[]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn matching_prefix_pattern_returns_empty() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host org.env.*\n  User admin\n");

        // Act
        let msgs = check("org.env.host", &[config]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn matching_broader_prefix_returns_empty() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host org.*\n  User admin\n");

        // Act
        let msgs = check("org.env.host", &[config]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn host_star_fallback_returns_empty() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host *\n  User admin\n");

        // Act
        let msgs = check("org.env.host", &[config]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn no_matching_pattern_returns_error() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host other.*\n  User admin\n");

        // Act
        let msgs = check("org.env.host", &[config]).unwrap();

        // Assert
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].level, CheckLevel::Error);
        assert!(msgs[0].text.contains("No wildcard Host pattern"));
    }

    #[test]
    fn has_host_pattern_finds_pattern() {
        assert!(has_host_pattern("Host org.*\n", "org.*"));
        assert!(has_host_pattern("Host *\n", "*"));
        assert!(!has_host_pattern("Host other.*\n", "org.*"));
    }

    #[test]
    fn has_host_pattern_multiple_patterns_on_line() {
        assert!(has_host_pattern("Host foo.* org.*\n", "org.*"));
    }
}
