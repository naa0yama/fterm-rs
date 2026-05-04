//! SSH environment file loading.
//!
//! Parses and applies environment variables from the file specified by
//! the `SSH_ENV` environment variable. This is used to restore SSH agent
//! settings (e.g. `SSH_AUTH_SOCK`, `SSH_AGENT_PID`) from a saved state.

use std::env;
use std::fs;

use tracing::debug;

/// Load environment variables from the `SSH_ENV` file if it exists.
///
/// The file is expected to contain lines like:
/// ```text
/// SSH_AUTH_SOCK=/tmp/ssh-xxx/agent.123; export SSH_AUTH_SOCK;
/// SSH_AGENT_PID=456; export SSH_AGENT_PID;
/// ```
///
/// Only `KEY=VALUE` assignments are processed; `export` directives and
/// other shell syntax are ignored.
pub fn load() {
    let path = match env::var("SSH_ENV") {
        Ok(p) if !p.is_empty() => p,
        _ => return,
    };

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            debug!(path = %path, error = %e, "could not read SSH_ENV file");
            return;
        }
    };

    debug!(path = %path, "loading SSH_ENV file");

    for line in content.lines() {
        let trimmed = line.trim();
        // Skip empty lines, comments, and bare "export" lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Handle "VAR=value; export VAR;" or "VAR=value"
        let assignment = trimmed.split(';').next().unwrap_or(trimmed).trim();
        if let Some((key, value)) = assignment.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                debug!(key = %key, "setting env var from SSH_ENV");
                // SAFETY: SSH_ENV loading runs early in the single-threaded
                // startup path, before any concurrent work begins.
                unsafe { env::set_var(key, value) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::env;

    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn load_sets_env_vars_from_file() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let env_file = tmp.path().join("ssh_env");
        std::fs::write(
            &env_file,
            "SSH_AUTH_SOCK=/tmp/ssh-test/agent.999; export SSH_AUTH_SOCK;\nSSH_AGENT_PID=42; export SSH_AGENT_PID;\n",
        )
        .unwrap();
        unsafe { env::set_var("SSH_ENV", env_file.to_str().unwrap()) };

        // Act
        load();

        // Assert
        assert_eq!(
            env::var("SSH_AUTH_SOCK").unwrap(),
            "/tmp/ssh-test/agent.999"
        );
        assert_eq!(env::var("SSH_AGENT_PID").unwrap(), "42");

        // Cleanup
        unsafe {
            env::remove_var("SSH_ENV");
            env::remove_var("SSH_AUTH_SOCK");
            env::remove_var("SSH_AGENT_PID");
        };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn load_does_nothing_without_ssh_env() {
        // Arrange
        unsafe { env::remove_var("SSH_ENV") };

        // Act & Assert — should not panic
        load();
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn load_handles_missing_file() {
        // Arrange
        unsafe { env::set_var("SSH_ENV", "/nonexistent/ssh_env") };

        // Act & Assert — should not panic
        load();

        // Cleanup
        unsafe { env::remove_var("SSH_ENV") };
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn load_skips_comments_and_empty_lines() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let env_file = tmp.path().join("ssh_env");
        std::fs::write(&env_file, "# comment\n\nSSH_TEST_VAR=hello\n").unwrap();
        unsafe { env::set_var("SSH_ENV", env_file.to_str().unwrap()) };

        // Act
        load();

        // Assert
        assert_eq!(env::var("SSH_TEST_VAR").unwrap(), "hello");

        // Cleanup
        unsafe {
            env::remove_var("SSH_ENV");
            env::remove_var("SSH_TEST_VAR");
        };
    }
}
