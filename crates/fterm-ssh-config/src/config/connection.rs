//! SSH connection info extraction via `ssh -G`.

use anyhow::{Context, Result};

use fterm_core::runner::CommandRunner;

/// Resolved SSH connection parameters for a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Remote user name.
    pub user: String,
    /// Resolved hostname or IP address.
    pub hostname: String,
    /// SSH port number.
    pub port: String,
}

/// Parse connection info from pre-resolved `ssh -G` output.
///
/// Returns `None` if any of the three required fields is missing.
#[must_use]
pub fn parse_connection_info(ssh_g_output: &str) -> Option<Info> {
    let mut user = None;
    let mut hostname = None;
    let mut port = None;

    for line in ssh_g_output.lines() {
        if let Some((key, value)) = line.split_once(' ') {
            let value = value.trim();
            match key.to_lowercase().as_str() {
                "user" => user = Some(String::from(value)),
                "hostname" => hostname = Some(String::from(value)),
                "port" => port = Some(String::from(value)),
                _ => {}
            }
        }
    }

    match (user, hostname, port) {
        (Some(u), Some(h), Some(p)) => Some(Info {
            user: u,
            hostname: h,
            port: p,
        }),
        _ => None,
    }
}

/// Extract connection info (user, hostname, port) for a host using `ssh -G`.
///
/// Returns `None` if any of the three required fields is missing from the output.
///
/// # Errors
/// Returns an error if the `ssh -G` command fails.
pub fn get_connection_info(
    runner: &dyn CommandRunner,
    host: &str,
    config_args: &[String],
) -> Result<Option<Info>> {
    let output = runner
        .ssh_resolve(host, config_args)
        .with_context(|| format!("Failed to resolve SSH config for host: {host}"))?;

    Ok(parse_connection_info(&output))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[test]
    fn parses_complete_connection_info() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "myhost",
            "user deploy\nhostname 10.0.0.1\nport 2222\nidentityfile ~/.ssh/id_rsa\n",
        );

        // Act
        let result = get_connection_info(&runner, "myhost", &[]).unwrap();

        // Assert
        let info = result.unwrap();
        assert_eq!(info.user, "deploy");
        assert_eq!(info.hostname, "10.0.0.1");
        assert_eq!(info.port, "2222");
    }

    #[test]
    fn returns_none_when_field_missing() {
        // Arrange
        let runner =
            MockCommandRunner::new().with_ssh_resolve("myhost", "user deploy\nhostname 10.0.0.1\n");

        // Act
        let result = get_connection_info(&runner, "myhost", &[]).unwrap();

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn case_insensitive_keys() {
        // Arrange
        let runner = MockCommandRunner::new()
            .with_ssh_resolve("myhost", "User deploy\nHostName 10.0.0.1\nPort 2222\n");

        // Act
        let result = get_connection_info(&runner, "myhost", &[]).unwrap();

        // Assert
        let info = result.unwrap();
        assert_eq!(info.user, "deploy");
        assert_eq!(info.hostname, "10.0.0.1");
        assert_eq!(info.port, "2222");
    }

    #[test]
    fn preserves_value_case() {
        // Arrange — values should NOT be lowercased (e.g. user "Deploy")
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "myhost",
            "user Deploy\nhostname MyServer.Example.COM\nport 2222\n",
        );

        // Act
        let result = get_connection_info(&runner, "myhost", &[]).unwrap();

        // Assert
        let info = result.unwrap();
        assert_eq!(info.user, "Deploy");
        assert_eq!(info.hostname, "MyServer.Example.COM");
    }

    #[test]
    fn extra_fields_ignored() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "myhost",
            "user deploy\nhostname 10.0.0.1\nport 2222\nidentityfile ~/.ssh/id_rsa\nforwardagent yes\n",
        );

        // Act
        let result = get_connection_info(&runner, "myhost", &[]).unwrap();

        // Assert
        let info = result.unwrap();
        assert_eq!(info.user, "deploy");
        assert_eq!(info.hostname, "10.0.0.1");
        assert_eq!(info.port, "2222");
    }

    #[test]
    fn duplicate_keys_uses_last() {
        // Arrange: same key appears twice, last value wins
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "myhost",
            "user first\nhostname 10.0.0.1\nport 22\nuser second\n",
        );

        // Act
        let result = get_connection_info(&runner, "myhost", &[]).unwrap();

        // Assert
        let info = result.unwrap();
        assert_eq!(info.user, "second");
    }

    #[test]
    fn handles_empty_output() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let result = get_connection_info(&runner, "myhost", &[]).unwrap();

        // Assert
        assert!(result.is_none());
    }
}
