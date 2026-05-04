//! SSH host extraction from config files.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing::debug;

/// Extract all non-wildcard host names from the given SSH config files.
///
/// Reads each file, finds lines starting with `Host `, extracts individual
/// host patterns, filters out wildcards (containing `*` or `?`), and returns
/// a sorted, deduplicated list.
///
/// # Errors
/// Returns an error if any config file cannot be read.
pub fn get_all(config_files: &[PathBuf]) -> Result<Vec<String>> {
    let mut hosts = Vec::new();

    for path in config_files {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        for line in content.lines() {
            let trimmed = line.trim();

            // Match "Host " prefix (case-insensitive, SSH accepts both)
            if let Some(rest) = trimmed
                .strip_prefix("Host ")
                .or_else(|| trimmed.strip_prefix("host "))
            {
                for pattern in rest.split_whitespace() {
                    if pattern.contains('*') || pattern.contains('?') {
                        debug!("Skipping wildcard host pattern: {pattern}");
                        continue;
                    }
                    hosts.push(String::from(pattern));
                }
            }
        }
    }

    hosts.sort();
    hosts.dedup();
    Ok(hosts)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn extracts_simple_hosts() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "Host alpha\n  HostName alpha.example.com\nHost beta\n",
        )
        .unwrap();

        // Act
        let result = get_all(&[config]).unwrap();

        // Assert
        assert_eq!(result, vec!["alpha", "beta"]);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn filters_wildcard_hosts() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "Host *\nHost ?.local\nHost real\n").unwrap();

        // Act
        let result = get_all(&[config]).unwrap();

        // Assert
        assert_eq!(result, vec!["real"]);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn handles_multiple_hosts_per_line() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "Host foo bar baz\n").unwrap();

        // Act
        let result = get_all(&[config]).unwrap();

        // Assert
        assert_eq!(result, vec!["bar", "baz", "foo"]);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn deduplicates_across_files() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config1 = dir.path().join("config1");
        let config2 = dir.path().join("config2");
        fs::write(&config1, "Host shared\nHost unique1\n").unwrap();
        fs::write(&config2, "Host shared\nHost unique2\n").unwrap();

        // Act
        let result = get_all(&[config1, config2]).unwrap();

        // Assert
        assert_eq!(result, vec!["shared", "unique1", "unique2"]);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn extracts_lowercase_host_directive() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "host lower-case\n  HostName lower.example.com\n").unwrap();

        // Act
        let result = get_all(&[config]).unwrap();

        // Assert
        assert_eq!(result, vec!["lower-case"]);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn extracts_mixed_case_host_directives() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "Host upper\n  HostName a.com\nhost lower\n  HostName b.com\n",
        )
        .unwrap();

        // Act
        let result = get_all(&[config]).unwrap();

        // Assert
        assert_eq!(result, vec!["lower", "upper"]);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn returns_empty_for_no_hosts() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "# Just comments\n\n").unwrap();

        // Act
        let result = get_all(&[config]).unwrap();

        // Assert
        assert!(result.is_empty());
    }
}
