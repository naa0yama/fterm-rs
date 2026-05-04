//! `ProxyJump` chain validation — circular references and reachability.

use anyhow::{Context, Result};
use tracing::debug;

use crate::validate::{basic, identity};
use fterm_core::check_types::{CheckLevel, CheckMessage};
use fterm_core::runner::CommandRunner;
use fterm_core::ssh_parse::parse_ssh_g_value;

/// Validate the `ProxyJump` chain for a host.
///
/// Checks for:
/// - Circular proxy references
/// - Proxy hosts that are not defined in config and don't look like valid
///   direct addresses
/// - Recursively validates basic/identity checks on proxy hosts that are in
///   config
///
/// # Errors
/// Returns an error if the SSH config cannot be resolved.
pub fn check(
    runner: &dyn CommandRunner,
    ssh_g_output: &str,
    host: &str,
    config_args: &[String],
    hosts_in_config: &[String],
    visited: &mut Vec<String>,
) -> Result<Vec<CheckMessage>> {
    let mut checked = std::collections::HashSet::new();
    check_inner(
        runner,
        Some(ssh_g_output),
        host,
        config_args,
        hosts_in_config,
        visited,
        &mut checked,
    )
}

/// Inner recursive check with a `checked` cache to avoid re-validating
/// the same proxy host across multiple chains.
fn check_inner(
    runner: &dyn CommandRunner,
    pre_resolved: Option<&str>,
    host: &str,
    config_args: &[String],
    hosts_in_config: &[String],
    visited: &mut Vec<String>,
    checked: &mut std::collections::HashSet<String>,
) -> Result<Vec<CheckMessage>> {
    let output = match pre_resolved {
        Some(o) => String::from(o),
        None => runner
            .ssh_resolve(host, config_args)
            .with_context(|| format!("proxyjump check: failed to resolve host {host}"))?,
    };

    let proxyjump = parse_ssh_g_value(&output, "proxyjump").unwrap_or_default();

    if proxyjump.is_empty() || proxyjump == "none" {
        return Ok(Vec::new());
    }

    let mut messages = Vec::new();

    for proxy in proxyjump.split(',') {
        let proxy = proxy.trim();
        if proxy.is_empty() {
            continue;
        }

        // Extract the bare hostname (strip user@ prefix if present)
        let proxy_host = proxy.rsplit_once('@').map_or(proxy, |(_, host)| host);

        // Circular reference check
        if visited.contains(&String::from(proxy_host)) {
            messages.push(CheckMessage {
                level: CheckLevel::Error,
                text: format!(
                    "[{host}] Circular ProxyJump detected: {} -> {proxy_host}",
                    visited.join(" -> ")
                ),
            });
            continue;
        }

        // Check if proxy host is defined in config
        let in_config = hosts_in_config.iter().any(|h| h == proxy_host);

        if in_config {
            // Skip if already fully checked in another chain
            if checked.contains(proxy_host) {
                debug!(proxy = %proxy_host, "already checked; skipping");
                continue;
            }

            // Recursively validate proxy hosts defined in config
            visited.push(String::from(proxy_host));

            // Resolve proxy host once for all sub-checks
            let proxy_output = match runner.ssh_resolve(proxy_host, config_args) {
                Ok(o) => o,
                Err(e) => {
                    debug!(proxy = %proxy_host, error = %e, "failed to resolve proxy host");
                    continue;
                }
            };

            // Basic checks on proxy
            messages.extend(basic::check(&proxy_output, proxy_host));

            // Identity checks on proxy
            match identity::check(runner, &proxy_output, proxy_host) {
                Ok(sub_msgs) => messages.extend(sub_msgs),
                Err(e) => {
                    debug!(proxy = %proxy_host, error = %e, "identity check on proxy failed");
                }
            }

            // Recursive ProxyJump check
            match check_inner(
                runner,
                Some(&proxy_output),
                proxy_host,
                config_args,
                hosts_in_config,
                visited,
                checked,
            ) {
                Ok(sub_msgs) => messages.extend(sub_msgs),
                Err(e) => {
                    debug!(proxy = %proxy_host, error = %e, "proxyjump recursion on proxy failed");
                }
            }

            checked.insert(String::from(proxy_host));
        } else if is_likely_direct_address(proxy_host) {
            // Allow user@host, IPv4 addresses, and simple hostnames
            debug!(host = %host, proxy = %proxy_host, "proxy not in config but looks like direct address");
        } else {
            messages.push(CheckMessage {
                level: CheckLevel::Error,
                text: format!("[{host}] ProxyJump host \"{proxy_host}\" is not defined in config"),
            });
        }
    }

    debug!(host = %host, message_count = messages.len(), "proxyjump check complete");
    Ok(messages)
}

/// Heuristic: check if a string looks like a direct address rather than a
/// config alias. Returns `true` for IPv4 addresses or simple hostnames
/// (no dots, or single-part names).
fn is_likely_direct_address(host: &str) -> bool {
    // IPv4 pattern
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok()) {
        return true;
    }

    // Simple hostname (single part, no dots)
    !host.contains('.')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use fterm_core::check_types::CheckLevel;
    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[test]
    fn no_proxyjump_returns_empty() {
        // Arrange
        let runner = MockCommandRunner::new();
        let output = "hostname example.com\nproxyjump none\n";
        let hosts = vec![String::from("myhost")];
        let mut visited = vec![String::from("myhost")];

        // Act
        let msgs = check(&runner, output, "myhost", &[], &hosts, &mut visited).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn circular_reference_returns_error() {
        // Arrange
        let host_a_output =
            "hostname a.example.com\nproxyjump host-b\nuser admin\nport 22\nidentitiesonly yes\n";
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "host-b",
            "hostname b.example.com\nproxyjump host-a\nuser admin\nport 22\nidentitiesonly yes\n",
        );
        let hosts = vec![String::from("host-a"), String::from("host-b")];
        let mut visited = vec![String::from("host-a")];

        // Act
        let msgs = check(&runner, host_a_output, "host-a", &[], &hosts, &mut visited).unwrap();

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("Circular"))
        );
    }

    #[test]
    fn unknown_proxy_not_direct_address_returns_error() {
        // Arrange
        let runner = MockCommandRunner::new();
        let output = "hostname example.com\nproxyjump org.env.bastion.weird\n";
        let hosts = vec![String::from("myhost")];
        let mut visited = vec![String::from("myhost")];

        // Act
        let msgs = check(&runner, output, "myhost", &[], &hosts, &mut visited).unwrap();

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("not defined"))
        );
    }

    #[test]
    fn ipv4_proxy_allowed() {
        // Arrange
        let runner = MockCommandRunner::new();
        let output = "hostname example.com\nproxyjump 192.168.1.1\n";
        let hosts = vec![String::from("myhost")];
        let mut visited = vec![String::from("myhost")];

        // Act
        let msgs = check(&runner, output, "myhost", &[], &hosts, &mut visited).unwrap();

        // Assert
        assert!(
            !msgs
                .iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("not defined")),
            "IPv4 address should be allowed: {msgs:?}"
        );
    }

    #[test]
    fn simple_hostname_proxy_allowed() {
        // Arrange
        let runner = MockCommandRunner::new();
        let output = "hostname example.com\nproxyjump bastion\n";
        let hosts = vec![String::from("myhost")];
        let mut visited = vec![String::from("myhost")];

        // Act
        let msgs = check(&runner, output, "myhost", &[], &hosts, &mut visited).unwrap();

        // Assert
        assert!(
            !msgs
                .iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("not defined")),
            "Simple hostname should be allowed: {msgs:?}"
        );
    }

    #[test]
    fn duplicate_proxy_across_chains_checked_once() {
        // Arrange — host-a and host-b both use shared-proxy
        let output_a = "hostname a.example.com\nproxyjump shared-proxy\nuser admin\nport 22\nidentitiesonly yes\n";
        let output_b = "hostname b.example.com\nproxyjump shared-proxy\nuser admin\nport 22\nidentitiesonly yes\n";
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "shared-proxy",
            "hostname proxy.example.com\nuser admin\nport 22\nidentitiesonly yes\n",
        );
        let hosts = vec![
            String::from("host-a"),
            String::from("host-b"),
            String::from("shared-proxy"),
        ];

        // Act — check host-a first, then host-b
        let mut visited_a = vec![String::from("host-a")];
        let msgs_a = check(&runner, output_a, "host-a", &[], &hosts, &mut visited_a).unwrap();

        let mut visited_b = vec![String::from("host-b")];
        let msgs_b = check(&runner, output_b, "host-b", &[], &hosts, &mut visited_b).unwrap();

        // Assert — both should succeed without errors
        assert!(
            !msgs_a.iter().any(|m| m.level == CheckLevel::Error),
            "host-a should have no errors: {msgs_a:?}"
        );
        assert!(
            !msgs_b.iter().any(|m| m.level == CheckLevel::Error),
            "host-b should have no errors: {msgs_b:?}"
        );
    }

    #[test]
    fn is_likely_direct_address_ipv4() {
        assert!(is_likely_direct_address("192.168.1.1"));
        assert!(is_likely_direct_address("10.0.0.1"));
    }

    #[test]
    fn is_likely_direct_address_simple_hostname() {
        assert!(is_likely_direct_address("bastion"));
        assert!(is_likely_direct_address("jumphost"));
    }

    #[test]
    fn is_likely_direct_address_rejects_fqdn() {
        assert!(!is_likely_direct_address("org.env.host"));
    }
}
