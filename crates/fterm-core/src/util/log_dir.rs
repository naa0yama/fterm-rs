//! Shared log directory prefix resolution.

use std::env;

use super::path::resolve_home;

/// Default prefix for fterm log files (relative to home).
pub const DEFAULT_LOG_DIR_PREFIX: &str = ".dotfiles/logs/tmux";

/// Get the log directory prefix from the environment or default.
///
/// Checks `FTERM_LOG_DIR_PREFIX` env var first, then falls back to
/// `$HOME/.dotfiles/logs/tmux`.
#[must_use]
pub fn get_prefix() -> String {
    env::var("FTERM_LOG_DIR_PREFIX").unwrap_or_else(|_| {
        let home = resolve_home();
        format!("{home}/{DEFAULT_LOG_DIR_PREFIX}")
    })
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
    fn returns_env_value() {
        // Arrange
        unsafe { env::set_var("FTERM_LOG_DIR_PREFIX", "/custom/logs") };

        // Act
        let prefix = get_prefix();

        // Assert
        assert_eq!(prefix, "/custom/logs");

        // Cleanup
        unsafe { env::remove_var("FTERM_LOG_DIR_PREFIX") };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn falls_back_to_default() {
        // Arrange
        unsafe { env::remove_var("FTERM_LOG_DIR_PREFIX") };

        // Act
        let prefix = get_prefix();

        // Assert
        assert!(prefix.ends_with(DEFAULT_LOG_DIR_PREFIX));
    }
}
