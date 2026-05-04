//! SSH agent key matching against host identity files.
//!
//! Matches identity files configured for a host (via `ssh -G`) against
//! keys loaded in the SSH agent, providing source attribution.

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::runner::CommandRunner;
use fterm_core::ssh_parse::parse_ssh_g_values;
use fterm_core::util::path::expand_tilde;

/// Get agent-loaded keys matching identity files from pre-resolved `ssh -G` output.
///
/// # Errors
/// Returns an error if `ssh-add -l` or `ssh-keygen -lf` fails.
pub fn get_matched_agent_keys_from_output(
    runner: &dyn CommandRunner,
    ssh_g_output: &str,
) -> Result<Vec<String>> {
    let identity_files = parse_ssh_g_values(ssh_g_output, "identityfile");

    if identity_files.is_empty() {
        return Ok(Vec::new());
    }

    let agent_result = runner
        .ssh_agent_list()
        .context("Failed to list SSH agent keys")?;

    if !agent_result.available || agent_result.keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut matched = Vec::new();

    for identity_file in &identity_files {
        let expanded = expand_tilde(identity_file);

        let fingerprint = match runner.ssh_keygen_fingerprint(&expanded) {
            Ok(fp) => fp,
            Err(e) => {
                debug!("Could not get fingerprint for {}: {e}", expanded.display());
                continue;
            }
        };

        let fp_trimmed = fingerprint.trim();
        let fp_hash = extract_fingerprint_hash(fp_trimmed);

        for agent_key in &agent_result.keys {
            let hashes_match = matches!(
                (extract_fingerprint_hash(agent_key), fp_hash),
                (Some(agent_hash), Some(file_hash)) if agent_hash == file_hash
            );
            if hashes_match {
                matched.push(format!("{agent_key} (from: {identity_file})"));
            }
        }
    }

    Ok(matched)
}

/// Get agent-loaded keys that match the identity files configured for a host.
///
/// For each `IdentityFile` found in the `ssh -G` output, computes its
/// fingerprint via `ssh-keygen -lf` and checks whether that fingerprint
/// appears in the `ssh-add -l` output. Returns matched keys formatted as
/// `"{agent_key_line} (from: {identity_file})"`.
///
/// # Errors
/// Returns an error if `ssh -G`, `ssh-add -l`, or `ssh-keygen -lf` fails.
pub fn get_matched_agent_keys(
    runner: &dyn CommandRunner,
    host: &str,
    config_args: &[String],
) -> Result<Vec<String>> {
    let ssh_output = runner
        .ssh_resolve(host, config_args)
        .with_context(|| format!("Failed to resolve SSH config for host: {host}"))?;

    get_matched_agent_keys_from_output(runner, &ssh_output)
}

/// Extract the fingerprint hash (e.g., `SHA256:xxx`) from an `ssh-keygen -lf`
/// or `ssh-add -l` output line.
fn extract_fingerprint_hash(line: &str) -> Option<&str> {
    // Lines look like: "2048 SHA256:abcdef comment (RSA)"
    line.split_whitespace().find(|part| part.contains(':'))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use serial_test::serial;

    use fterm_core::runner::AgentListResult;
    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn matches_agent_key_to_identity_file() {
        // Arrange
        let home = std::env::var("HOME").unwrap();
        let key_path = format!("{home}/.ssh/id_ed25519");

        let runner = MockCommandRunner::new()
            .with_ssh_resolve("myhost", "identityfile ~/.ssh/id_ed25519\n")
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![String::from("256 SHA256:abc123 user@host (ED25519)")],
            })
            .with_fingerprint(&key_path, "256 SHA256:abc123 user@host (ED25519)");

        // Act
        let result = get_matched_agent_keys(&runner, "myhost", &[]).unwrap();

        // Assert
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("SHA256:abc123"));
        assert!(result[0].contains("(from: ~/.ssh/id_ed25519)"));
    }

    #[test]
    fn returns_empty_when_no_identity_files() {
        // Arrange
        let runner = MockCommandRunner::new()
            .with_ssh_resolve("myhost", "user deploy\nhostname example.com\n")
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![String::from("256 SHA256:abc123 user@host (ED25519)")],
            });

        // Act
        let result = get_matched_agent_keys(&runner, "myhost", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn returns_empty_when_agent_unavailable() {
        // Arrange
        let runner =
            MockCommandRunner::new().with_ssh_resolve("myhost", "identityfile ~/.ssh/id_ed25519\n");

        // Act
        let result = get_matched_agent_keys(&runner, "myhost", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn skips_unmatched_fingerprints() {
        // Arrange
        let home = std::env::var("HOME").unwrap();
        let key_path = format!("{home}/.ssh/id_ed25519");

        let runner = MockCommandRunner::new()
            .with_ssh_resolve("myhost", "identityfile ~/.ssh/id_ed25519\n")
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![String::from("256 SHA256:abc123 user@host (ED25519)")],
            })
            .with_fingerprint(&key_path, "256 SHA256:different user@host (ED25519)");

        // Act
        let result = get_matched_agent_keys(&runner, "myhost", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn handles_multiple_identity_files() {
        // Arrange
        let home = std::env::var("HOME").unwrap();
        let key1 = format!("{home}/.ssh/id_ed25519");
        let key2 = format!("{home}/.ssh/id_rsa");

        let runner = MockCommandRunner::new()
            .with_ssh_resolve(
                "myhost",
                "identityfile ~/.ssh/id_ed25519\nidentityfile ~/.ssh/id_rsa\n",
            )
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![
                    String::from("256 SHA256:ed25519fp user@host (ED25519)"),
                    String::from("2048 SHA256:rsafp user@host (RSA)"),
                ],
            })
            .with_fingerprint(&key1, "256 SHA256:ed25519fp user@host (ED25519)")
            .with_fingerprint(&key2, "2048 SHA256:rsafp user@host (RSA)");

        // Act
        let result = get_matched_agent_keys(&runner, "myhost", &[]).unwrap();

        // Assert
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn skips_identity_files_with_no_fingerprint() {
        // Arrange — no fingerprint registered, so ssh-keygen will return empty
        let runner = MockCommandRunner::new()
            .with_ssh_resolve("myhost", "identityfile /nonexistent/key\n")
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![String::from("256 SHA256:abc123 user@host (ED25519)")],
            });

        // Act
        let result = get_matched_agent_keys(&runner, "myhost", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }
}
