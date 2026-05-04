//! ASCII art banner generation for SSH connect/disconnect and SCP results.

use std::fmt::Write;

use chrono::Local;

// ANSI color codes
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Common banner parameters shared between SSH and SCP banners.
#[derive(Debug)]
pub struct BannerParams<'a> {
    /// Path to the session log file.
    pub log_path: &'a str,
    /// SSH config details (e.g. `ProxyJump`, `IdentityFile`).
    pub ssh_details: &'a [String],
    /// Agent keys matched to the host.
    pub agent_keys: &'a [String],
}

/// Generate an ASCII art banner showing SSH connection info.
///
/// Includes timestamp, config name, `user@hostname:port`, log path, and
/// optionally SSH config details and agent keys when the provided slices
/// are non-empty.
#[must_use]
pub fn ssh_connect_banner(
    config_name: &str,
    user: &str,
    hostname: &str,
    port: &str,
    params: &BannerParams<'_>,
) -> String {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut buf = String::new();

    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}╔══════════════════════════════════════╗{RESET}"
    );
    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}║        fterm SSH Connected           ║{RESET}"
    );
    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}╚══════════════════════════════════════╝{RESET}"
    );
    let _ = writeln!(buf, "  {BOLD}Timestamp:{RESET}  {CYAN}{timestamp}{RESET}");
    let _ = writeln!(buf, "  {BOLD}Config:{RESET}     {CYAN}{config_name}{RESET}");
    let _ = writeln!(
        buf,
        "  {BOLD}Target:{RESET}     {CYAN}{user}@{hostname}:{port}{RESET}"
    );
    let _ = writeln!(
        buf,
        "  {BOLD}Log:{RESET}        {CYAN}{log_path}{RESET}",
        log_path = params.log_path
    );

    append_details_and_keys(&mut buf, params.ssh_details, params.agent_keys);

    buf
}

/// Append SSH config details and agent keys sections to a banner buffer.
fn append_details_and_keys(buf: &mut String, ssh_details: &[String], agent_keys: &[String]) {
    if !ssh_details.is_empty() {
        let _ = writeln!(buf, "  {BOLD}SSH Config:{RESET}");
        for detail in ssh_details {
            let _ = writeln!(buf, "    {CYAN}{detail}{RESET}");
        }
    }

    if !agent_keys.is_empty() {
        let _ = writeln!(buf, "  {BOLD}Agent Keys:{RESET}");
        for key in agent_keys {
            let _ = writeln!(buf, "    {CYAN}{key}{RESET}");
        }
    }
}

/// Generate a disconnect banner showing session summary.
///
/// Includes timestamp, `user@hostname`, and session duration.
#[must_use]
pub fn ssh_disconnect_banner(
    user: &str,
    hostname: &str,
    duration_str: &str,
    log_path: &str,
) -> String {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut buf = String::new();

    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}╔══════════════════════════════════════╗{RESET}"
    );
    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}║       fterm SSH Disconnected         ║{RESET}"
    );
    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}╚══════════════════════════════════════╝{RESET}"
    );
    let _ = writeln!(buf, "  {BOLD}Timestamp:{RESET}  {CYAN}{timestamp}{RESET}");
    let _ = writeln!(
        buf,
        "  {BOLD}Target:{RESET}     {CYAN}{user}@{hostname}{RESET}"
    );
    let _ = writeln!(
        buf,
        "  {BOLD}Duration:{RESET}   {CYAN}{duration_str}{RESET}"
    );
    let _ = writeln!(buf, "  {BOLD}Log:{RESET}        {CYAN}{log_path}{RESET}");

    buf
}

/// Generate an ASCII art banner showing SCP connection info before transfer.
///
/// Includes timestamp, target hosts, log path, and optionally
/// SSH config details and agent keys.
#[must_use]
pub fn scp_connect_banner(hosts: &str, params: &BannerParams<'_>) -> String {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut buf = String::new();

    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}╔══════════════════════════════════════╗{RESET}"
    );
    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}║        fterm SCP Transfer            ║{RESET}"
    );
    let _ = writeln!(
        buf,
        "{GREEN}{BOLD}╚══════════════════════════════════════╝{RESET}"
    );
    let _ = writeln!(buf, "  {BOLD}Timestamp:{RESET}  {CYAN}{timestamp}{RESET}");
    let _ = writeln!(buf, "  {BOLD}Target:{RESET}     {CYAN}{hosts}{RESET}");
    let _ = writeln!(
        buf,
        "  {BOLD}Log:{RESET}        {CYAN}{log_path}{RESET}",
        log_path = params.log_path
    );

    append_details_and_keys(&mut buf, params.ssh_details, params.agent_keys);

    buf
}

/// Generate a banner showing SCP transfer results.
///
/// Displays the list of target hosts and whether the transfer succeeded
/// (green) or failed (red).
#[must_use]
pub fn scp_result_banner(
    hosts: &[String],
    success: bool,
    duration_str: &str,
    log_path: &str,
) -> String {
    let (status, color) = if success {
        ("SUCCESS", GREEN)
    } else {
        ("FAILED", RED)
    };
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");

    let mut buf = String::new();

    let _ = writeln!(
        buf,
        "{color}{BOLD}╔══════════════════════════════════════╗{RESET}"
    );
    let _ = writeln!(
        buf,
        "{color}{BOLD}║         fterm SCP Result             ║{RESET}"
    );
    let _ = writeln!(
        buf,
        "{color}{BOLD}╚══════════════════════════════════════╝{RESET}"
    );
    let _ = writeln!(
        buf,
        "  {BOLD}Status:{RESET}     {color}{BOLD}{status}{RESET}"
    );
    let _ = writeln!(buf, "  {BOLD}Timestamp:{RESET}  {CYAN}{timestamp}{RESET}");
    let _ = writeln!(
        buf,
        "  {BOLD}Duration:{RESET}   {CYAN}{duration_str}{RESET}"
    );
    let _ = writeln!(buf, "  {BOLD}Log:{RESET}        {CYAN}{log_path}{RESET}");

    if !hosts.is_empty() {
        let _ = writeln!(buf, "  {BOLD}Hosts:{RESET}");
        for host in hosts {
            let _ = writeln!(buf, "    {CYAN}{host}{RESET}");
        }
    }

    buf
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| String::from(*a)).collect()
    }

    fn empty_params(log_path: &str) -> BannerParams<'_> {
        BannerParams {
            log_path,
            ssh_details: &[],
            agent_keys: &[],
        }
    }

    // --- ssh_connect_banner ---

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_contains_header() {
        // Arrange
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "22",
            &empty_params("/tmp/log"),
        );

        // Act & Assert
        assert!(banner.contains("fterm SSH Connected"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_contains_config_name() {
        // Arrange & Act
        let banner = ssh_connect_banner(
            "my-alias",
            "alice",
            "10.0.0.1",
            "22",
            &empty_params("/tmp/log"),
        );

        // Assert
        assert!(banner.contains("Config:"));
        assert!(banner.contains("my-alias"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_contains_target() {
        // Arrange & Act
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "2222",
            &empty_params("/tmp/log"),
        );

        // Assert
        assert!(banner.contains("alice@server1:2222"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_contains_log_path() {
        // Arrange & Act
        let banner = ssh_connect_banner(
            "host",
            "bob",
            "host",
            "22",
            &empty_params("/var/log/fterm/session.log"),
        );

        // Assert
        assert!(banner.contains("/var/log/fterm/session.log"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_shows_ssh_details() {
        // Arrange
        let details = s(&["ProxyJump bastion", "ForwardAgent yes"]);

        // Act
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "22",
            &BannerParams {
                log_path: "/tmp/log",
                ssh_details: &details,
                agent_keys: &[],
            },
        );

        // Assert
        assert!(banner.contains("SSH Config:"));
        assert!(banner.contains("ProxyJump bastion"));
        assert!(banner.contains("ForwardAgent yes"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_hides_ssh_details_when_empty() {
        // Arrange & Act
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "22",
            &empty_params("/tmp/log"),
        );

        // Assert
        assert!(!banner.contains("SSH Config:"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_shows_agent_keys() {
        // Arrange
        let keys = s(&["SHA256:abc123 user@laptop", "SHA256:def456 deploy-key"]);

        // Act
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "22",
            &BannerParams {
                log_path: "/tmp/log",
                ssh_details: &[],
                agent_keys: &keys,
            },
        );

        // Assert
        assert!(banner.contains("Agent Keys:"));
        assert!(banner.contains("SHA256:abc123 user@laptop"));
        assert!(banner.contains("SHA256:def456 deploy-key"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_hides_agent_keys_when_empty() {
        // Arrange & Act
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "22",
            &empty_params("/tmp/log"),
        );

        // Assert
        assert!(!banner.contains("Agent Keys:"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_contains_timestamp_label() {
        // Arrange & Act
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "22",
            &empty_params("/tmp/log"),
        );

        // Assert
        assert!(banner.contains("Timestamp:"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn connect_banner_uses_ansi_colors() {
        // Arrange & Act
        let banner = ssh_connect_banner(
            "server1",
            "alice",
            "server1",
            "22",
            &empty_params("/tmp/log"),
        );

        // Assert — verify ANSI escape sequences are present
        assert!(banner.contains("\x1b[32m")); // green
        assert!(banner.contains("\x1b[36m")); // cyan
        assert!(banner.contains("\x1b[0m")); // reset
    }

    // --- ssh_disconnect_banner ---

    #[cfg_attr(miri, ignore)]
    #[test]
    fn disconnect_banner_contains_header() {
        // Arrange & Act
        let banner = ssh_disconnect_banner("alice", "server1", "5m30s", "/tmp/log");

        // Assert
        assert!(banner.contains("fterm SSH Disconnected"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn disconnect_banner_contains_target() {
        // Arrange & Act
        let banner = ssh_disconnect_banner("alice", "server1", "1h2m3s", "/tmp/log");

        // Assert
        assert!(banner.contains("alice@server1"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn disconnect_banner_contains_duration() {
        // Arrange & Act
        let banner = ssh_disconnect_banner("bob", "host", "3d 2h15m0s", "/tmp/log");

        // Assert
        assert!(banner.contains("Duration:"));
        assert!(banner.contains("3d 2h15m0s"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn disconnect_banner_contains_timestamp_label() {
        // Arrange & Act
        let banner = ssh_disconnect_banner("alice", "server1", "0s", "/tmp/log");

        // Assert
        assert!(banner.contains("Timestamp:"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn disconnect_banner_uses_ansi_colors() {
        // Arrange & Act
        let banner = ssh_disconnect_banner("alice", "server1", "10s", "/tmp/log");

        // Assert
        assert!(banner.contains("\x1b[32m")); // green
        assert!(banner.contains("\x1b[36m")); // cyan
    }

    // --- scp_result_banner ---

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_banner_success() {
        // Arrange
        let hosts = s(&["host1", "host2"]);

        // Act
        let banner = scp_result_banner(&hosts, true, "5s", "/tmp/log");

        // Assert
        assert!(banner.contains("SUCCESS"));
        assert!(banner.contains("\x1b[32m")); // green
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_banner_failure() {
        // Arrange
        let hosts = s(&["host1"]);

        // Act
        let banner = scp_result_banner(&hosts, false, "3s", "/tmp/log");

        // Assert
        assert!(banner.contains("FAILED"));
        assert!(banner.contains("\x1b[31m")); // red
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_banner_contains_header() {
        // Arrange & Act
        let banner = scp_result_banner(&s(&["host1"]), true, "0s", "/tmp/log");

        // Assert
        assert!(banner.contains("fterm SCP Result"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_banner_lists_hosts() {
        // Arrange
        let hosts = s(&["alpha", "bravo", "charlie"]);

        // Act
        let banner = scp_result_banner(&hosts, true, "5s", "/tmp/log");

        // Assert
        assert!(banner.contains("Hosts:"));
        assert!(banner.contains("alpha"));
        assert!(banner.contains("bravo"));
        assert!(banner.contains("charlie"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_banner_hides_hosts_when_empty() {
        // Arrange & Act
        let banner = scp_result_banner(&[], true, "0s", "");

        // Assert
        assert!(!banner.contains("Hosts:"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_banner_success_no_red() {
        // Arrange & Act
        let banner = scp_result_banner(&[], true, "0s", "");

        // Assert — success banner should not contain red
        assert!(!banner.contains("\x1b[31m"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_banner_failure_no_green_in_header() {
        // Arrange & Act
        let banner = scp_result_banner(&[], false, "0s", "");

        // Assert — failure banner header uses red, not green
        assert!(!banner.contains("\x1b[32m"));
    }

    // --- scp_connect_banner ---

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_connect_banner_contains_header() {
        // Arrange & Act
        let banner = scp_connect_banner("host1_host2", &empty_params("/tmp/log"));

        // Assert
        assert!(banner.contains("fterm SCP Transfer"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_connect_banner_contains_target() {
        // Arrange & Act
        let banner = scp_connect_banner("host1_host2", &empty_params("/tmp/log"));

        // Assert
        assert!(banner.contains("host1_host2"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_connect_banner_contains_log_path() {
        // Arrange & Act
        let banner = scp_connect_banner("host", &empty_params("/var/log/scp.log"));

        // Assert
        assert!(banner.contains("/var/log/scp.log"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_connect_banner_shows_ssh_details() {
        // Arrange
        let details = s(&["ProxyJump bastion", "ForwardAgent yes"]);

        // Act
        let banner = scp_connect_banner(
            "host",
            &BannerParams {
                log_path: "/tmp/log",
                ssh_details: &details,
                agent_keys: &[],
            },
        );

        // Assert
        assert!(banner.contains("SSH Config:"));
        assert!(banner.contains("ProxyJump bastion"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn scp_connect_banner_shows_agent_keys() {
        // Arrange
        let keys = s(&["SHA256:abc123 user@laptop"]);

        // Act
        let banner = scp_connect_banner(
            "host",
            &BannerParams {
                log_path: "/tmp/log",
                ssh_details: &[],
                agent_keys: &keys,
            },
        );

        // Assert
        assert!(banner.contains("Agent Keys:"));
        assert!(banner.contains("SHA256:abc123 user@laptop"));
    }
}
