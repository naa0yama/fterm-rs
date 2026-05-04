//! Basic SSH config field validation for a single host.

use tracing::debug;

use fterm_core::check_types::{CheckLevel, CheckMessage};
use fterm_core::ssh_parse::{parse_ssh_g_value, parse_ssh_g_values};

/// Check basic SSH config fields for a host from pre-resolved `ssh -G` output.
///
/// Validates: `HostName`, `User`, `Port`, `IdentitiesOnly`, `IdentityFile`.
/// This is a pure function — it does not spawn subprocesses.
#[must_use]
pub fn check(ssh_g_output: &str, host: &str) -> Vec<CheckMessage> {
    let mut messages = Vec::new();

    // HostName
    let hostname = parse_ssh_g_value(ssh_g_output, "hostname").unwrap_or_default();
    if hostname.is_empty() || (hostname == host && host.split('.').count() >= 3) {
        messages.push(CheckMessage {
            level: CheckLevel::Error,
            text: format!(
                "[{host}] HostName is empty or equals the host alias with 3+ dot-separated parts"
            ),
        });
    }

    // User
    let user = parse_ssh_g_value(ssh_g_output, "user").unwrap_or_default();
    if user.is_empty() {
        messages.push(CheckMessage {
            level: CheckLevel::Error,
            text: format!("[{host}] User is not set"),
        });
    }

    // Port
    let port = parse_ssh_g_value(ssh_g_output, "port").unwrap_or_default();
    if port.is_empty() {
        messages.push(CheckMessage {
            level: CheckLevel::Error,
            text: format!("[{host}] Port is not set"),
        });
    }

    // IdentitiesOnly
    let identities_only = parse_ssh_g_value(ssh_g_output, "identitiesonly").unwrap_or_default();
    if identities_only != "yes" {
        messages.push(CheckMessage {
            level: CheckLevel::Warn,
            text: format!(
                "[{host}] IdentitiesOnly is not \"yes\" (current: \"{identities_only}\")"
            ),
        });
    }

    // IdentityFile
    let identity_files = parse_ssh_g_values(ssh_g_output, "identityfile");
    if identity_files.is_empty() {
        messages.push(CheckMessage {
            level: CheckLevel::Warn,
            text: format!("[{host}] No IdentityFile configured"),
        });
    }

    debug!(host = %host, message_count = messages.len(), "basic check complete");
    messages
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use fterm_core::check_types::CheckLevel;

    use super::*;

    #[test]
    fn all_fields_valid() {
        // Arrange
        let output = "hostname example.com\nuser admin\nport 22\nidentitiesonly yes\nidentityfile ~/.ssh/id_rsa\n";

        // Act
        let msgs = check(output, "myhost");

        // Assert
        assert!(msgs.is_empty(), "expected no messages, got: {msgs:?}");
    }

    #[test]
    fn missing_user_returns_error() {
        // Arrange
        let output =
            "hostname example.com\nport 22\nidentitiesonly yes\nidentityfile ~/.ssh/id_rsa\n";

        // Act
        let msgs = check(output, "myhost");

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("User"))
        );
    }

    #[test]
    fn hostname_equals_alias_with_dots_returns_error() {
        // Arrange
        let output = "hostname org.env.host\nuser admin\nport 22\nidentitiesonly yes\nidentityfile ~/.ssh/id_rsa\n";

        // Act
        let msgs = check(output, "org.env.host");

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("HostName"))
        );
    }

    #[test]
    fn identities_only_not_yes_returns_warn() {
        // Arrange
        let output = "hostname example.com\nuser admin\nport 22\nidentitiesonly no\nidentityfile ~/.ssh/id_rsa\n";

        // Act
        let msgs = check(output, "myhost");

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Warn && m.text.contains("IdentitiesOnly"))
        );
    }

    #[test]
    fn empty_hostname_returns_error() {
        // Arrange
        let output =
            "hostname \nuser admin\nport 22\nidentitiesonly yes\nidentityfile ~/.ssh/id_rsa\n";

        // Act
        let msgs = check(output, "myhost");

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("HostName"))
        );
    }

    #[test]
    fn missing_port_returns_error() {
        // Arrange
        let output =
            "hostname example.com\nuser admin\nidentitiesonly yes\nidentityfile ~/.ssh/id_rsa\n";

        // Act
        let msgs = check(output, "myhost");

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("Port"))
        );
    }

    #[test]
    fn hostname_with_two_parts_no_error() {
        // Arrange
        let output = "hostname example.com\nuser admin\nport 22\nidentitiesonly yes\nidentityfile ~/.ssh/id_rsa\n";

        // Act
        let msgs = check(output, "example.com");

        // Assert
        assert!(
            !msgs
                .iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("HostName")),
            "expected no HostName error for 2-part hostname, got: {msgs:?}"
        );
    }

    #[test]
    fn all_fields_missing_returns_multiple_messages() {
        // Act
        let msgs = check("", "myhost");

        // Assert: hostname, user, port, identitiesonly, identityfile => at least 4 messages
        assert!(
            msgs.len() >= 4,
            "expected at least 4 messages for empty output, got {}: {msgs:?}",
            msgs.len()
        );
    }

    #[test]
    fn no_identity_file_returns_warn() {
        // Arrange
        let output = "hostname example.com\nuser admin\nport 22\nidentitiesonly yes\n";

        // Act
        let msgs = check(output, "myhost");

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Warn && m.text.contains("IdentityFile"))
        );
    }
}
