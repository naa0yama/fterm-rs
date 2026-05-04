//! SSH config block extraction for a specific host.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Maximum number of lines to include after the `Host` line.
const MAX_BLOCK_LINES: usize = 20;

/// Extract the SSH config block for a given host from config files.
///
/// Searches each config file for a `Host <name>` line (exact match) and
/// returns up to 20 subsequent lines until the next `Host`/`Match` directive
/// or end of file.
///
/// # Errors
///
/// Returns an error if any config file cannot be read.
pub fn extract_host(host: &str, config_files: &[PathBuf]) -> Result<String> {
    let mut result = String::new();

    for path in config_files {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;

        if let Some(block) = find_host_block(host, &content) {
            if !result.is_empty() {
                result.push('\n');
            }
            let _ = write!(result, "# {}\n{block}", path.display());
        }
    }

    if result.is_empty() {
        return Ok(format!("No config found for {host}"));
    }

    Ok(result)
}

/// Find the config block for `host` within a single file's content.
fn find_host_block(host: &str, content: &str) -> Option<String> {
    let mut lines = content.lines();
    let mut capturing = false;
    let mut block = String::new();
    let mut count = 0;

    loop {
        let Some(line) = lines.next() else {
            break;
        };

        let trimmed = line.trim();

        if capturing {
            // Stop at next Host/Match directive
            if is_host_or_match_line(trimmed) {
                break;
            }
            if count >= MAX_BLOCK_LINES {
                block.push_str("  ...\n");
                break;
            }
            block.push_str(line);
            block.push('\n');
            count = count.saturating_add(1);
        } else if is_exact_host_line(trimmed, host) {
            block.push_str(line);
            block.push('\n');
            capturing = true;
        }
    }

    if block.is_empty() { None } else { Some(block) }
}

/// Check if a line is a `Host` directive that exactly matches the given host.
fn is_exact_host_line(trimmed: &str, host: &str) -> bool {
    trimmed
        .strip_prefix("Host ")
        .or_else(|| trimmed.strip_prefix("host "))
        .is_some_and(|rest| rest.split_whitespace().any(|h| h == host))
}

/// Check if a line starts a new `Host` or `Match` block.
fn is_host_or_match_line(trimmed: &str) -> bool {
    trimmed.starts_with("Host ")
        || trimmed.starts_with("host ")
        || trimmed.starts_with("Match ")
        || trimmed.starts_with("match ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::fmt::Write as _;
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn extracts_single_host_block() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "Host alpha\n  HostName alpha.example.com\n  User admin\n  Port 22\n\nHost beta\n  HostName beta.example.com\n",
        )
        .unwrap();

        // Act
        let result = extract_host("alpha", &[config]).unwrap();

        // Assert
        assert!(result.contains("Host alpha"));
        assert!(result.contains("HostName alpha.example.com"));
        assert!(result.contains("User admin"));
        assert!(!result.contains("Host beta"));
        assert!(!result.contains("beta.example.com"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn returns_fallback_when_host_not_found() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "Host other\n  HostName other.example.com\n").unwrap();

        // Act
        let result = extract_host("missing", &[config]).unwrap();

        // Assert
        assert!(result.contains("No config found for missing"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn extracts_from_multiple_files() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config1 = dir.path().join("config1");
        let config2 = dir.path().join("config2");
        fs::write(&config1, "Host alpha\n  HostName alpha.example.com\n").unwrap();
        fs::write(&config2, "Host alpha\n  User admin\n").unwrap();

        // Act
        let result = extract_host("alpha", &[config1, config2]).unwrap();

        // Assert
        assert!(result.contains("HostName alpha.example.com"));
        assert!(result.contains("User admin"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn stops_at_next_host_directive() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "Host target\n  HostName target.example.com\nHost next\n  HostName next.example.com\n",
        )
        .unwrap();

        // Act
        let result = extract_host("target", &[config]).unwrap();

        // Assert
        assert!(result.contains("target.example.com"));
        assert!(!result.contains("next.example.com"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn stops_at_match_directive() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "Host target\n  HostName target.example.com\nMatch exec \"true\"\n  ProxyJump proxy\n",
        )
        .unwrap();

        // Act
        let result = extract_host("target", &[config]).unwrap();

        // Assert
        assert!(result.contains("target.example.com"));
        assert!(!result.contains("ProxyJump"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn truncates_at_max_lines() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        let mut content = String::from("Host target\n");
        for i in 0..30 {
            let _ = writeln!(content, "  Option{i} value{i}");
        }
        fs::write(&config, &content).unwrap();

        // Act
        let result = extract_host("target", &[config]).unwrap();

        // Assert
        assert!(result.contains("..."));
        // Should have Host line + 20 option lines + "..."
        let line_count = result.lines().count();
        // file header + Host line + 20 lines + "..."
        assert!(line_count <= 24, "got {line_count} lines");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn extracts_lowercase_host_block() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "host target\n  HostName target.example.com\n  User admin\n",
        )
        .unwrap();

        // Act
        let result = extract_host("target", &[config]).unwrap();

        // Assert
        assert!(result.contains("host target"));
        assert!(result.contains("HostName target.example.com"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn stops_at_lowercase_host_directive() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "Host target\n  HostName target.example.com\nhost next\n  HostName next.example.com\n",
        )
        .unwrap();

        // Act
        let result = extract_host("target", &[config]).unwrap();

        // Assert
        assert!(result.contains("target.example.com"));
        assert!(!result.contains("next.example.com"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn stops_at_lowercase_match_directive() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "Host target\n  HostName target.example.com\nmatch exec \"true\"\n  ProxyJump proxy\n",
        )
        .unwrap();

        // Act
        let result = extract_host("target", &[config]).unwrap();

        // Assert
        assert!(result.contains("target.example.com"));
        assert!(!result.contains("ProxyJump"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn handles_multi_host_line() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "Host foo bar baz\n  HostName shared.example.com\n").unwrap();

        // Act
        let result = extract_host("bar", &[config]).unwrap();

        // Assert
        assert!(result.contains("shared.example.com"));
    }
}
