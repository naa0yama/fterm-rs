//! Path utilities.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// Resolve the user's home directory.
///
/// On MSYS2 (detected via `MSYSTEM`), prefers `USERPROFILE`.
/// Falls back to `HOME`, then `/root`.
#[must_use]
pub fn resolve_home() -> String {
    if std::env::var("MSYSTEM").is_ok()
        && let Ok(profile) = std::env::var("USERPROFILE")
    {
        return profile;
    }
    std::env::var("HOME").unwrap_or_else(|_| String::from("/root"))
}

/// Expand a leading `~` to the user's home directory.
#[must_use]
pub fn expand_tilde(path: &str) -> PathBuf {
    path.strip_prefix("~/")
        .or_else(|| path.strip_prefix('~'))
        .map_or_else(
            || PathBuf::from(path),
            |rest| PathBuf::from(resolve_home()).join(rest),
        )
}

/// Return the MSYS2-compatible HOME path if running on MSYS2.
///
/// Runs `cygpath -m $USERPROFILE` to produce a mixed-mode path suitable for
/// Windows OpenSSH. Returns `None` when not on MSYS2 or if `USERPROFILE`
/// is unset.
// NOTEST(env): requires MSYSTEM env var and cygpath binary (MSYS2-only)
#[cfg_attr(coverage_nightly, coverage(off))]
#[must_use]
pub fn msys2_home() -> Option<String> {
    if std::env::var("MSYSTEM").is_err() {
        return None;
    }
    let profile = std::env::var("USERPROFILE").ok()?;
    let output = Command::new("cygpath")
        .args(["-m", &profile])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        None
    }
}

/// Known Windows OpenSSH directories (MSYS2 path format).
const WIN_SSH_DIRS: &[&str] = &[
    "/c/Windows/System32/OpenSSH", // Windows 10+ built-in
    "/c/Program Files/OpenSSH",    // manually installed
];

/// Resolve the path for an SSH-related command on MSYS2.
///
/// Searches known Windows OpenSSH directories for `{name}.exe`.
/// Returns `None` if not on MSYS2 or no executable found.
// NOTEST(env): requires MSYSTEM env var and Windows OpenSSH paths (MSYS2-only)
#[cfg_attr(coverage_nightly, coverage(off))]
#[must_use]
pub fn resolve_win_ssh_command(name: &str) -> Option<String> {
    if std::env::var("MSYSTEM").is_err() {
        return None;
    }
    for dir in WIN_SSH_DIRS {
        let full = format!("{dir}/{name}.exe");
        if Path::new(&full).exists() {
            return Some(full);
        }
    }
    None
}

/// Convert a path to Windows mixed format via `cygpath -m`.
///
/// On MSYS2 (detected via `MSYSTEM`), runs `cygpath -m` to produce
/// a Windows-style path (e.g. `C:/Users/user/.ssh/config`).
/// On non-MSYS2 environments, returns the path as-is.
///
/// # Errors
///
/// Returns an error if `cygpath` cannot be spawned on MSYS2.
// NOTEST(env): MSYS2 branch requires cygpath; non-MSYS2 path is tested via existing tests
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn to_win_mixed(path: &Path) -> Result<String> {
    if std::env::var("MSYSTEM").is_ok() {
        let output = Command::new("cygpath")
            .args(["-m", &path.display().to_string()])
            .output()
            .context("failed to run cygpath")?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Ok(path.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::undocumented_unsafe_blocks)]

    use serial_test::serial;

    use super::*;

    // -- resolve_home tests --

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn resolve_home_uses_home_by_default() {
        // Arrange
        unsafe {
            std::env::remove_var("MSYSTEM");
            std::env::set_var("HOME", "/home/testuser");
        };

        // Act
        let home = resolve_home();

        // Assert
        assert_eq!(home, "/home/testuser");

        // Cleanup
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn resolve_home_prefers_userprofile_on_msys2() {
        // Arrange
        unsafe {
            std::env::set_var("MSYSTEM", "MINGW64");
            std::env::set_var("USERPROFILE", "C:\\Users\\testuser");
            std::env::set_var("HOME", "/home/testuser");
        };

        // Act
        let home = resolve_home();

        // Assert
        assert_eq!(home, "C:\\Users\\testuser");

        // Cleanup
        unsafe {
            std::env::remove_var("MSYSTEM");
            std::env::remove_var("USERPROFILE");
            std::env::remove_var("HOME");
        };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn resolve_home_falls_back_to_home_on_msys2_without_userprofile() {
        // Arrange
        unsafe {
            std::env::set_var("MSYSTEM", "MINGW64");
            std::env::remove_var("USERPROFILE");
            std::env::set_var("HOME", "/home/fallback");
        };

        // Act
        let home = resolve_home();

        // Assert
        assert_eq!(home, "/home/fallback");

        // Cleanup
        unsafe {
            std::env::remove_var("MSYSTEM");
            std::env::remove_var("HOME");
        };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn resolve_home_falls_back_to_root() {
        // Arrange
        unsafe {
            std::env::remove_var("MSYSTEM");
            std::env::remove_var("USERPROFILE");
            std::env::remove_var("HOME");
        };

        // Act
        let home = resolve_home();

        // Assert
        assert_eq!(home, "/root");
    }

    // -- expand_tilde tests --

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn expands_tilde_to_home() {
        // Arrange
        unsafe {
            std::env::remove_var("MSYSTEM");
            std::env::set_var("HOME", "/home/testuser");
        };

        // Act
        let expanded = expand_tilde("~/foo/bar");

        // Assert
        assert_eq!(expanded, PathBuf::from("/home/testuser/foo/bar"));

        // Cleanup
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn no_tilde_returns_as_is() {
        // Arrange / Act
        let expanded = expand_tilde("/absolute/path");

        // Assert
        assert_eq!(expanded, PathBuf::from("/absolute/path"));
    }

    // -- to_windows_path tests --

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn to_win_mixed_returns_as_is_without_msystem() {
        // Arrange
        unsafe { std::env::remove_var("MSYSTEM") };
        let path = Path::new("/home/user/.ssh/config");

        // Act
        let result = to_win_mixed(path).unwrap();

        // Assert
        assert_eq!(result, "/home/user/.ssh/config");
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn expand_tilde_bare_tilde_returns_home() {
        // Arrange
        unsafe {
            std::env::remove_var("MSYSTEM");
            std::env::set_var("HOME", "/home/bareuser");
        };

        // Act
        let expanded = expand_tilde("~");

        // Assert
        assert_eq!(expanded, PathBuf::from("/home/bareuser"));

        // Cleanup
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn msys2_home_returns_none_without_msystem() {
        // Arrange
        unsafe { std::env::remove_var("MSYSTEM") };

        // Act
        let result = msys2_home();

        // Assert — no MSYSTEM means no MSYS2 home
        assert!(result.is_none());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn resolve_win_ssh_command_returns_none_without_msystem() {
        // Arrange
        unsafe { std::env::remove_var("MSYSTEM") };

        // Act
        let result = resolve_win_ssh_command("ssh");

        // Assert — no MSYSTEM means no Windows SSH command
        assert!(result.is_none());
    }
}
