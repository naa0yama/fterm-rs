//! Duplicate host detection across SSH config files.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::check_types::{CheckLevel, CheckMessage};

/// Detect duplicate `Host` entries across config files.
///
/// Reads all provided config files and collects `Host` entries with their
/// source location (file + line number). Wildcards (`*`, `?`, `!`) are
/// skipped. Reports a warning for each duplicated host alias.
///
/// # Errors
/// Returns an error if a config file cannot be read.
pub fn check(config_files: &[PathBuf]) -> Result<Vec<CheckMessage>> {
    let mut host_locations: HashMap<String, Vec<(PathBuf, usize)>> = HashMap::new();

    for path in config_files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("duplicate check: failed to read {}", path.display()))?;

        for (line_idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            let host_line = if let Some(rest) = trimmed.strip_prefix("Host ") {
                rest
            } else if let Some(rest) = trimmed.strip_prefix("host ") {
                rest
            } else {
                continue;
            };

            // Each Host line may define multiple aliases
            for alias in host_line.split_whitespace() {
                // Skip wildcard patterns
                if alias.contains('*') || alias.contains('?') || alias.starts_with('!') {
                    continue;
                }
                host_locations
                    .entry(String::from(alias))
                    .or_default()
                    .push((path.clone(), line_idx.saturating_add(1)));
            }
        }
    }

    let mut messages = Vec::new();
    for (alias, locations) in &host_locations {
        if locations.len() > 1 {
            let locs: Vec<String> = locations
                .iter()
                .map(|(p, l)| format!("{}:{l}", p.display()))
                .collect();
            messages.push(CheckMessage {
                level: CheckLevel::Warn,
                text: format!("Duplicate Host \"{alias}\" defined at: {}", locs.join(", ")),
            });
        }
    }

    debug!(duplicate_count = messages.len(), "duplicate check complete");
    Ok(messages)
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
    fn no_duplicates_returns_empty() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host alpha\nHost beta\n");

        // Act
        let msgs = check(&[config]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn duplicate_in_same_file_returns_warn() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(
            &dir,
            "config",
            "Host myhost\n  User a\n\nHost myhost\n  User b\n",
        );

        // Act
        let msgs = check(&[config]).unwrap();

        // Assert
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].level, CheckLevel::Warn);
        assert!(msgs[0].text.contains("myhost"));
    }

    #[test]
    fn duplicate_across_files_returns_warn() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let c1 = write_config(&dir, "config1", "Host shared\n  User a\n");
        let c2 = write_config(&dir, "config2", "Host shared\n  User b\n");

        // Act
        let msgs = check(&[c1, c2]).unwrap();

        // Assert
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].text.contains("shared"));
    }

    #[test]
    fn wildcards_are_skipped() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host *\n\nHost *\n");

        // Act
        let msgs = check(&[config]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn negation_patterns_skipped() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host !excluded\nHost !excluded\n");

        // Act
        let msgs = check(&[config]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn multiple_aliases_on_one_line() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(&dir, "config", "Host alpha beta\nHost alpha\n");

        // Act
        let msgs = check(&[config]).unwrap();

        // Assert
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].text.contains("alpha"));
    }
}
