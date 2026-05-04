//! SSH config detail extraction via `ssh -G`.
//!
//! Extracts proxy, identity, forwarding, and agent settings from
//! the resolved SSH configuration for a given host.

use anyhow::{Context, Result};

use fterm_core::runner::CommandRunner;

/// Parse SSH config details from pre-resolved `ssh -G` output.
///
/// Returns a list of formatted strings such as `"ProxyJump bastion"`.
#[must_use]
pub fn parse(ssh_g_output: &str) -> Vec<String> {
    let mut details = Vec::new();

    for line in ssh_g_output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some((key, value)) = trimmed.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        let key_lower = key.to_lowercase();

        match key_lower.as_str() {
            "proxyjump" => {
                if !value.eq_ignore_ascii_case("none") {
                    details.push(format!("ProxyJump {value}"));
                }
            }
            "proxycommand" => {
                if !value.eq_ignore_ascii_case("none") {
                    details.push(format!("ProxyCommand {value}"));
                }
            }
            "identityfile" => {
                details.push(format!("IdentityFile {value}"));
            }
            "identitiesonly" => {
                details.push(format!("IdentitiesOnly {value}"));
            }
            "forwardagent" => {
                if value.eq_ignore_ascii_case("yes") {
                    details.push(format!("ForwardAgent {value}"));
                }
            }
            "localforward" => {
                details.push(format!("LocalForward {value}"));
            }
            "remoteforward" => {
                details.push(format!("RemoteForward {value}"));
            }
            "dynamicforward" => {
                if !value.eq_ignore_ascii_case("none") {
                    details.push(format!("DynamicForward {value}"));
                }
            }
            _ => {}
        }
    }

    details
}

/// Extract detailed SSH configuration settings for a host using `ssh -G`.
///
/// Returns a list of formatted strings such as `"ProxyJump bastion"` or
/// `"IdentityFile ~/.ssh/id_ed25519"`.
///
/// The following keys are extracted (case-insensitive):
/// - `ProxyJump` (skipped if "none")
/// - `ProxyCommand` (skipped if "none")
/// - `IdentityFile` (all values)
/// - `IdentitiesOnly` (all values)
/// - `ForwardAgent` (only if "yes")
/// - `LocalForward` (all values)
/// - `RemoteForward` (all values)
/// - `DynamicForward` (skipped if "none")
///
/// # Errors
/// Returns an error if the `ssh -G` command fails.
pub fn get(runner: &dyn CommandRunner, host: &str, config_args: &[String]) -> Result<Vec<String>> {
    let output = runner
        .ssh_resolve(host, config_args)
        .with_context(|| format!("Failed to resolve SSH config for host: {host}"))?;

    Ok(parse(&output))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[test]
    fn extracts_proxy_jump() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve("host", "proxyjump bastion\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert_eq!(result, vec!["ProxyJump bastion"]);
    }

    #[test]
    fn skips_proxy_jump_none() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve("host", "proxyjump none\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn extracts_identity_file() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "host",
            "identityfile ~/.ssh/id_ed25519\nidentityfile ~/.ssh/id_rsa\n",
        );

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert_eq!(
            result,
            vec![
                "IdentityFile ~/.ssh/id_ed25519",
                "IdentityFile ~/.ssh/id_rsa",
            ]
        );
    }

    #[test]
    fn extracts_forward_agent_only_if_yes() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve("host", "forwardagent yes\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert_eq!(result, vec!["ForwardAgent yes"]);
    }

    #[test]
    fn skips_forward_agent_no() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve("host", "forwardagent no\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn extracts_proxy_command() {
        // Arrange
        let runner = MockCommandRunner::new()
            .with_ssh_resolve("host", "proxycommand ssh -W %h:%p bastion\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert_eq!(result, vec!["ProxyCommand ssh -W %h:%p bastion"]);
    }

    #[test]
    fn skips_proxy_command_none() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve("host", "proxycommand none\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn extracts_forwarding_settings() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "host",
            "localforward 8080 localhost:80\nremoteforward 9090 localhost:90\ndynamicforward 1080\n",
        );

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert_eq!(
            result,
            vec![
                "LocalForward 8080 localhost:80",
                "RemoteForward 9090 localhost:90",
                "DynamicForward 1080",
            ]
        );
    }

    #[test]
    fn skips_dynamic_forward_none() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve("host", "dynamicforward none\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn extracts_identities_only() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve("host", "identitiesonly yes\n");

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert_eq!(result, vec!["IdentitiesOnly yes"]);
    }

    #[test]
    fn handles_mixed_output() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "host",
            "user deploy\nhostname 10.0.0.1\nport 22\nproxyjump bastion\nidentityfile ~/.ssh/id_ed25519\nforwardagent yes\nidentitiesonly yes\n",
        );

        // Act
        let result = get(&runner, "host", &[]).unwrap();

        // Assert
        assert_eq!(
            result,
            vec![
                "ProxyJump bastion",
                "IdentityFile ~/.ssh/id_ed25519",
                "ForwardAgent yes",
                "IdentitiesOnly yes",
            ]
        );
    }
}
