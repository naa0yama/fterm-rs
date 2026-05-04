//! SSH home directory resolution.

use std::path::PathBuf;

use anyhow::{Context, Result};

use fterm_core::util::path;
use fterm_core::util::path::resolve_home;

/// Returns the path to the user's SSH configuration directory.
///
/// Priority: `FSSH_SSH_CONF_DIR` env var → `resolve_home()/.ssh`.
/// On MSYS2, `resolve_home()` prefers `USERPROFILE` over `HOME`.
#[must_use]
pub fn get_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FSSH_SSH_CONF_DIR") {
        return PathBuf::from(dir);
    }
    let home = resolve_home();
    PathBuf::from(home).join(".ssh")
}

/// Build `-F` config args for SSH commands.
///
/// Returns `-F {config_path}` when a custom SSH directory is configured
/// (via `FSSH_SSH_CONF_DIR`) or when running on MSYS2.
/// Returns an empty vec when using the default `~/.ssh` directory on
/// non-MSYS2 systems (SSH reads it automatically).
///
/// # Errors
///
/// Returns an error if the config path cannot be converted on MSYS2.
pub fn build_config_args() -> Result<Vec<String>> {
    if path::resolve_win_ssh_command("ssh").is_some() {
        let config_path = get_dir().join("config");
        let win_path = path::to_win_mixed(&config_path).context("failed to convert config path")?;
        Ok(vec![win_path])
    } else if std::env::var("FSSH_SSH_CONF_DIR").is_ok() {
        let config_path = get_dir().join("config");
        Ok(vec![config_path.to_string_lossy().into_owned()])
    } else {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::undocumented_unsafe_blocks)]

    use serial_test::serial;

    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn returns_ssh_directory_under_home() {
        // Arrange
        unsafe {
            std::env::remove_var("FSSH_SSH_CONF_DIR");
            std::env::remove_var("MSYSTEM");
        };
        let home = std::env::var("HOME").unwrap();

        // Act
        let result = get_dir();

        // Assert
        assert_eq!(result, PathBuf::from(home).join(".ssh"));
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn returns_custom_dir_from_env() {
        // Arrange
        unsafe { std::env::set_var("FSSH_SSH_CONF_DIR", "/custom/ssh") };

        // Act
        let result = get_dir();

        // Assert
        assert_eq!(result, PathBuf::from("/custom/ssh"));

        // Cleanup
        unsafe { std::env::remove_var("FSSH_SSH_CONF_DIR") };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn build_config_args_returns_path_when_custom_dir_set() {
        // Arrange
        unsafe {
            std::env::set_var("FSSH_SSH_CONF_DIR", "/custom/ssh");
            std::env::remove_var("MSYSTEM");
        };

        // Act
        let result = build_config_args().unwrap();

        // Assert
        assert_eq!(result, vec!["/custom/ssh/config"]);

        // Cleanup
        unsafe { std::env::remove_var("FSSH_SSH_CONF_DIR") };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn build_config_args_returns_empty_when_default_dir() {
        // Arrange
        unsafe {
            std::env::remove_var("FSSH_SSH_CONF_DIR");
            std::env::remove_var("MSYSTEM");
        };

        // Act
        let result = build_config_args().unwrap();

        // Assert
        assert!(result.is_empty());
    }
}
