//! `ControlMaster` directory creation and validation.

use std::path::Path;

use tracing::debug;

use fterm_core::check_types::{CheckLevel, CheckMessage};

/// Ensure the `ControlMaster` socket directory exists.
///
/// Target: `{ssh_home}/conf.d/cm`. If it doesn't exist, attempts to create
/// it with permissions `0o700`.
#[must_use]
pub fn check(ssh_home: &Path) -> Vec<CheckMessage> {
    let cm_dir = ssh_home.join("conf.d").join("cm");

    if cm_dir.exists() {
        debug!(path = %cm_dir.display(), "cm directory already exists");
        return Vec::new();
    }

    debug!(path = %cm_dir.display(), "creating cm directory");

    if let Err(e) = std::fs::create_dir_all(&cm_dir) {
        return vec![CheckMessage {
            level: CheckLevel::Error,
            text: format!(
                "Failed to create ControlMaster directory {}: {e}",
                cm_dir.display()
            ),
        }];
    }

    // Set permissions to 0o700 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        if let Err(e) = std::fs::set_permissions(&cm_dir, perms) {
            return vec![CheckMessage {
                level: CheckLevel::Warn,
                text: format!(
                    "Created cm directory but failed to set permissions to 0700 on {}: {e}",
                    cm_dir.display()
                ),
            }];
        }
    }

    debug!(path = %cm_dir.display(), "cm directory created successfully");
    vec![CheckMessage {
        level: CheckLevel::Info,
        text: format!("Created ControlMaster directory: {}", cm_dir.display()),
    }]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn creates_cm_directory_returns_info_message() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let ssh_home = dir.path();

        // Act
        let msgs = check(ssh_home);

        // Assert
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].level, CheckLevel::Info);
        assert!(msgs[0].text.contains("Created ControlMaster directory"));
        assert!(ssh_home.join("conf.d").join("cm").exists());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn existing_cm_directory_returns_empty() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let cm_dir = dir.path().join("conf.d").join("cm");
        std::fs::create_dir_all(&cm_dir).unwrap();

        // Act
        let msgs = check(dir.path());

        // Assert
        assert!(msgs.is_empty());
    }

    #[cfg(unix)]
    #[cfg_attr(miri, ignore)]
    #[test]
    fn read_only_parent_returns_error() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let ssh_home = dir.path().join("ssh_home");
        std::fs::create_dir_all(&ssh_home).unwrap();
        let readonly_perms = std::fs::Permissions::from_mode(0o444);
        std::fs::set_permissions(&ssh_home, readonly_perms).unwrap();

        // Act
        let msgs = check(&ssh_home);

        // Assert (restore permissions before asserting so cleanup succeeds)
        let restore_perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&ssh_home, restore_perms).unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].level, CheckLevel::Error);
        assert!(msgs[0].text.contains("Failed to create"));
    }

    #[cfg(unix)]
    #[cfg_attr(miri, ignore)]
    #[test]
    fn created_directory_has_correct_permissions() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let ssh_home = dir.path();

        // Act
        let msgs = check(ssh_home);

        // Assert — only Info message (creation success), no errors/warnings
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].level, CheckLevel::Info);
        let cm_dir = ssh_home.join("conf.d").join("cm");
        let perms = std::fs::metadata(&cm_dir).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o700);
    }
}
