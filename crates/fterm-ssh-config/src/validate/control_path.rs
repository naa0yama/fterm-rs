//! `ControlPath` directory validation.

use tracing::debug;

use fterm_core::check_types::{CheckLevel, CheckMessage};
use fterm_core::ssh_parse::parse_ssh_g_value;
use fterm_core::util::path::expand_tilde;

/// Validate the `ControlPath` directory for a host from pre-resolved `ssh -G` output.
///
/// Skips validation when `ControlMaster` is `"no"` / empty or `ControlPath`
/// is `"none"` / empty. Otherwise warns if the parent directory does not
/// exist or is not writable.
///
/// This is a pure function — it does not spawn subprocesses.
#[must_use]
pub fn check(ssh_g_output: &str, host: &str) -> Vec<CheckMessage> {
    let control_master = parse_ssh_g_value(ssh_g_output, "controlmaster").unwrap_or_default();
    if control_master.is_empty() || control_master == "no" {
        return Vec::new();
    }

    let control_path = parse_ssh_g_value(ssh_g_output, "controlpath").unwrap_or_default();
    if control_path.is_empty() || control_path == "none" {
        return Vec::new();
    }

    let expanded = expand_tilde(&control_path);
    let Some(dir) = expanded.parent() else {
        return Vec::new();
    };

    let mut messages = Vec::new();

    if !dir.exists() {
        messages.push(CheckMessage {
            level: CheckLevel::Warn,
            text: format!(
                "[{host}] ControlPath directory does not exist: {}",
                dir.display()
            ),
        });
    } else if !is_writable(dir) {
        messages.push(CheckMessage {
            level: CheckLevel::Warn,
            text: format!(
                "[{host}] ControlPath directory is not writable: {}",
                dir.display()
            ),
        });
    }

    debug!(host = %host, message_count = messages.len(), "control_path check complete");
    messages
}

/// Check if a directory is writable by the effective user.
#[cfg(unix)]
fn is_writable(path: &std::path::Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::W_OK).is_ok()
}

#[cfg(not(unix))]
fn is_writable(path: &std::path::Path) -> bool {
    path.metadata().is_ok_and(|m| !m.permissions().readonly())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serial_test::serial;

    use fterm_core::check_types::CheckLevel;

    use super::*;

    #[test]
    fn control_master_disabled_skips() {
        // Act
        let msgs = check(
            "controlmaster no\ncontrolpath /tmp/ssh-%r@%h:%p\n",
            "myhost",
        );

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn control_path_none_skips() {
        // Act
        let msgs = check("controlmaster auto\ncontrolpath none\n", "myhost");

        // Assert
        assert!(msgs.is_empty());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn missing_directory_returns_warn() {
        // Act
        let msgs = check(
            "controlmaster auto\ncontrolpath /nonexistent/dir/ssh-%r@%h:%p\n",
            "myhost",
        );

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Warn && m.text.contains("does not exist"))
        );
    }

    #[test]
    fn control_master_empty_skips() {
        // Act
        let msgs = check("controlmaster \ncontrolpath /tmp/ssh-%r@%h:%p\n", "myhost");

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn control_path_empty_skips() {
        // Act
        let msgs = check("controlmaster auto\ncontrolpath \n", "myhost");

        // Assert
        assert!(msgs.is_empty());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn tilde_path_expands_correctly() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let ssh_dir = home.join(".ssh").join("cm");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        // SAFETY: test runs single-threaded; env var is restored after test.
        unsafe { std::env::set_var("HOME", home.as_os_str()) };

        // Act
        let msgs = check(
            "controlmaster auto\ncontrolpath ~/.ssh/cm/ssh-%r@%h:%p\n",
            "myhost",
        );

        // Assert
        assert!(msgs.is_empty(), "expected no messages: {msgs:?}");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn existing_directory_no_warn() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let control_path = format!("{}/ssh-%r@%h:%p", dir.path().display());
        let output = format!("controlmaster auto\ncontrolpath {control_path}\n");

        // Act
        let msgs = check(&output, "myhost");

        // Assert
        assert!(msgs.is_empty(), "expected no messages: {msgs:?}");
    }
}
