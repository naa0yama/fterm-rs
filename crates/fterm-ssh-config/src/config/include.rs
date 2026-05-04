//! SSH config `Include` directive resolution.
//!
//! Recursively resolves `Include` directives from SSH configuration files,
//! expanding globs and detecting cycles.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::util::path;

/// Recursively resolve all `Include` directives starting from `config_path`.
///
/// Returns a list of all config files (including `config_path` itself) in
/// the order they are encountered. Cycles are detected and skipped.
///
/// # Path resolution rules
/// - `~` prefix: expanded to `$HOME`
/// - `/` prefix: treated as absolute
/// - Otherwise: relative to `ssh_home`
///
/// # Errors
/// Returns an error if any referenced config file cannot be read or if glob
/// patterns are invalid.
pub fn resolve_included_files(config_path: &Path, ssh_home: &Path) -> Result<Vec<PathBuf>> {
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    resolve_recursive(config_path, ssh_home, &mut visited, &mut result)
        .with_context(|| format!("failed to resolve includes from {}", config_path.display()))?;
    Ok(result)
}

/// Internal recursive resolver.
fn resolve_recursive(
    config_path: &Path,
    ssh_home: &Path,
    visited: &mut HashSet<PathBuf>,
    result: &mut Vec<PathBuf>,
) -> Result<()> {
    let canonical = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());

    if !visited.insert(canonical.clone()) {
        debug!("Skipping already-visited config: {}", canonical.display());
        return Ok(());
    }

    result.push(config_path.to_path_buf());

    let content = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read SSH config: {}", config_path.display()))?;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Case-insensitive match for "Include"
        let lower = trimmed.to_lowercase();
        if let Some(rest) = lower.strip_prefix("include") {
            // Must be followed by whitespace
            if !rest.starts_with(' ') && !rest.starts_with('\t') {
                continue;
            }

            // Extract the original (non-lowercased) pattern value.
            // OpenSSH supports multiple space-separated patterns on one line.
            let patterns_str = trimmed[7..].trim();
            for pattern_str in patterns_str.split_whitespace() {
                let expanded = expand_include_pattern(pattern_str, ssh_home)?;
                for p in expanded {
                    resolve_recursive(&p, ssh_home, visited, result).with_context(|| {
                        format!("failed to resolve includes from {}", p.display())
                    })?;
                }
            }
        }
    }

    Ok(())
}

/// Expand a single include pattern to a list of matching paths.
fn expand_include_pattern(pattern: &str, ssh_home: &Path) -> Result<Vec<PathBuf>> {
    let resolved = match pattern.strip_prefix('~') {
        Some(rest) => {
            let home = path::resolve_home();
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            PathBuf::from(home).join(rest)
        }
        None if pattern.starts_with('/') => PathBuf::from(pattern),
        None => ssh_home.join(pattern),
    };

    let resolved_str = resolved
        .to_str()
        .with_context(|| format!("Non-UTF8 path: {}", resolved.display()))?;

    let paths: Vec<PathBuf> = glob::glob(resolved_str)
        .with_context(|| format!("Invalid glob pattern: {resolved_str}"))?
        .filter_map(|entry| match entry {
            Ok(p) => Some(p),
            Err(e) => {
                debug!("Glob entry error: {e}");
                None
            }
        })
        .collect();

    Ok(paths)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn resolves_single_config_without_includes() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "Host example\n  HostName example.com\n").unwrap();

        // Act
        let result = resolve_included_files(&config, dir.path()).unwrap();

        // Assert
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], config);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn resolves_relative_include() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let ssh_home = dir.path();
        let included = ssh_home.join("conf.d");
        fs::create_dir_all(&included).unwrap();

        let sub_config = included.join("work");
        fs::write(&sub_config, "Host work\n  HostName work.example.com\n").unwrap();

        let main_config = ssh_home.join("config");
        fs::write(&main_config, "Include conf.d/work\n").unwrap();

        // Act
        let result = resolve_included_files(&main_config, ssh_home).unwrap();

        // Assert
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], main_config);
        assert_eq!(result[1], sub_config);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn resolves_glob_include() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let ssh_home = dir.path();
        let conf_dir = ssh_home.join("conf.d");
        fs::create_dir_all(&conf_dir).unwrap();

        fs::write(conf_dir.join("a.conf"), "Host a\n").unwrap();
        fs::write(conf_dir.join("b.conf"), "Host b\n").unwrap();

        let main_config = ssh_home.join("config");
        fs::write(&main_config, "Include conf.d/*.conf\n").unwrap();

        // Act
        let result = resolve_included_files(&main_config, ssh_home).unwrap();

        // Assert
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], main_config);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn detects_cycles() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let ssh_home = dir.path();

        let config_a = ssh_home.join("config_a");
        let config_b = ssh_home.join("config_b");

        fs::write(&config_a, "Include config_b\n").unwrap();
        fs::write(&config_b, "Include config_a\n").unwrap();

        // Act
        let result = resolve_included_files(&config_a, ssh_home).unwrap();

        // Assert — no infinite loop, both files present exactly once
        assert_eq!(result.len(), 2);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn skips_comments_and_empty_lines() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::write(
            &config,
            "# This is a comment\n\n  # indented comment\nHost test\n",
        )
        .unwrap();

        // Act
        let result = resolve_included_files(&config, dir.path()).unwrap();

        // Assert
        assert_eq!(result.len(), 1);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn case_insensitive_include_keyword() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let ssh_home = dir.path();

        let sub = ssh_home.join("sub");
        fs::write(&sub, "Host sub\n").unwrap();

        let config = ssh_home.join("config");
        fs::write(&config, "INCLUDE sub\n").unwrap();

        // Act
        let result = resolve_included_files(&config, ssh_home).unwrap();

        // Assert
        assert_eq!(result.len(), 2);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn handles_absolute_include_path() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let ssh_home = dir.path();

        let abs_config = dir.path().join("absolute.conf");
        fs::write(&abs_config, "Host abs\n").unwrap();

        let main_config = ssh_home.join("config");
        fs::write(&main_config, format!("Include {}\n", abs_config.display())).unwrap();

        // Act
        let result = resolve_included_files(&main_config, ssh_home).unwrap();

        // Assert
        assert_eq!(result.len(), 2);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn resolves_multiple_patterns_on_one_line() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let ssh_home = dir.path();

        let sub_a = ssh_home.join("a.conf");
        fs::write(&sub_a, "Host alpha\n").unwrap();
        let sub_b = ssh_home.join("b.conf");
        fs::write(&sub_b, "Host beta\n").unwrap();

        let config = ssh_home.join("config");
        fs::write(&config, "Include a.conf b.conf\n").unwrap();

        // Act
        let result = resolve_included_files(&config, ssh_home).unwrap();

        // Assert — main config + both included files
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], config);
        assert_eq!(result[1], sub_a);
        assert_eq!(result[2], sub_b);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn ignores_nonexistent_glob_matches() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let ssh_home = dir.path();
        let config = ssh_home.join("config");
        fs::write(&config, "Include nonexistent/*.conf\n").unwrap();

        // Act
        let result = resolve_included_files(&config, ssh_home).unwrap();

        // Assert — only the main config
        assert_eq!(result.len(), 1);
    }
}
