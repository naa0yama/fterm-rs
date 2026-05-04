//! External command runner trait and associated types.
//!
//! Provides a trait-based interface for running external commands, enabling
//! mock injection in tests. The production implementation (`RealCommandRunner`)
//! lives in the `fterm` binary crate.

// `CommandRunner` and `MockCommandRunner` intentionally include "Runner" to
// match the module name for clarity in external usage (fterm_core::runner::CommandRunner).
#![allow(clippy::module_name_repetitions)]

use std::path::Path;

use anyhow::Result;

/// Output from an external command execution.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Process exit code (0 = success).
    pub exit_code: i32,
    /// Captured stdout content.
    pub stdout: String,
    /// Captured stderr content.
    pub stderr: String,
}

/// Result of `ssh-add -l` listing agent keys.
#[derive(Debug, Clone)]
pub struct AgentListResult {
    /// Whether the agent is available.
    pub available: bool,
    /// Raw output lines from `ssh-add -l`.
    pub keys: Vec<String>,
}

/// Abstraction for external command execution.
///
/// Implementations can run real commands or return mock responses for testing.
pub trait CommandRunner {
    /// Run a command with a timeout.
    ///
    /// # Errors
    /// Returns an error if the command cannot be spawned or times out.
    fn run(&self, cmd: &str, args: &[&str], timeout_secs: u64) -> Result<CommandOutput>;

    /// Resolve SSH config for a host via `ssh -G`.
    ///
    /// # Errors
    /// Returns an error if `ssh -G` fails.
    fn ssh_resolve(&self, host: &str, config_args: &[String]) -> Result<String>;

    /// Run a command interactively, inheriting stdin/stdout/stderr.
    ///
    /// Unlike [`run`](CommandRunner::run), this does **not** capture output.
    /// Use this for commands that need terminal access (e.g. `tmux attach`).
    ///
    /// The default implementation delegates to [`run`](CommandRunner::run) and
    /// returns the exit code, which is suitable for non-interactive contexts
    /// (e.g. tests).
    ///
    /// # Errors
    /// Returns an error if the command cannot be spawned.
    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<i32> {
        self.run(cmd, args, 0).map(|output| output.exit_code)
    }

    /// List agent keys via `ssh-add -l`.
    ///
    /// # Errors
    /// Returns an error if the agent is unreachable.
    fn ssh_agent_list(&self) -> Result<AgentListResult>;

    /// Get key fingerprint via `ssh-keygen -lf`.
    ///
    /// # Errors
    /// Returns an error if the key file is invalid.
    fn ssh_keygen_fingerprint(&self, path: &Path) -> Result<String>;
}

// ---------------------------------------------------------------------------
// MockCommandRunner
// ---------------------------------------------------------------------------

/// Mock implementation of [`CommandRunner`] for unit tests.
///
/// Allows pre-registering responses keyed by command string. Unregistered
/// commands return a default success response.
///
/// Available when compiling tests or when the `testutil` feature is enabled.
#[cfg(any(test, feature = "testutil"))]
#[derive(Debug, Default)]
pub struct MockCommandRunner {
    /// Keyed by `"cmd arg1 arg2 …"`.
    run_responses: std::sync::Mutex<std::collections::HashMap<String, CommandOutput>>,
    /// Keyed by `"cmd arg1 arg2 …"` — exit code only.
    interactive_responses: std::sync::Mutex<std::collections::HashMap<String, i32>>,
    /// Keyed by host name. `Err(msg)` simulates a failed resolve.
    ssh_resolve_responses:
        std::sync::Mutex<std::collections::HashMap<String, std::result::Result<String, String>>>,
    /// Single response for `ssh_agent_list`.
    agent_list_response: std::sync::Mutex<Option<AgentListResult>>,
    /// Keyed by path string. `Err(msg)` simulates a failed fingerprint.
    fingerprint_responses:
        std::sync::Mutex<std::collections::HashMap<String, std::result::Result<String, String>>>,
}

#[cfg(any(test, feature = "testutil"))]
impl MockCommandRunner {
    /// Create an empty mock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a response for a specific command invocation.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_run_response(self, key: &str, output: CommandOutput) -> Self {
        #[allow(clippy::unwrap_used)]
        self.run_responses
            .lock()
            .unwrap()
            .insert(String::from(key), output);
        self
    }

    /// Register a response for an interactive command invocation.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_interactive_response(self, key: &str, exit_code: i32) -> Self {
        #[allow(clippy::unwrap_used)]
        self.interactive_responses
            .lock()
            .unwrap()
            .insert(String::from(key), exit_code);
        self
    }

    /// Register a successful response for `ssh_resolve`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_ssh_resolve(self, host: &str, output: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.ssh_resolve_responses
            .lock()
            .unwrap()
            .insert(String::from(host), Ok(String::from(output)));
        self
    }

    /// Register an error response for `ssh_resolve`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_ssh_resolve_error(self, host: &str, error_msg: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.ssh_resolve_responses
            .lock()
            .unwrap()
            .insert(String::from(host), Err(String::from(error_msg)));
        self
    }

    /// Register a response for `ssh_agent_list`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_agent_list(self, result: AgentListResult) -> Self {
        #[allow(clippy::unwrap_used)]
        {
            *self.agent_list_response.lock().unwrap() = Some(result);
        }
        self
    }

    /// Register a successful response for `ssh_keygen_fingerprint`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_fingerprint(self, path: &str, fingerprint: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.fingerprint_responses
            .lock()
            .unwrap()
            .insert(String::from(path), Ok(String::from(fingerprint)));
        self
    }

    /// Register an error response for `ssh_keygen_fingerprint`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_fingerprint_error(self, path: &str, error_msg: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.fingerprint_responses
            .lock()
            .unwrap()
            .insert(String::from(path), Err(String::from(error_msg)));
        self
    }

    fn command_key(cmd: &str, args: &[&str]) -> String {
        if args.is_empty() {
            String::from(cmd)
        } else {
            format!("{cmd} {}", args.join(" "))
        }
    }
}

#[cfg(any(test, feature = "testutil"))]
impl CommandRunner for MockCommandRunner {
    fn run(&self, cmd: &str, args: &[&str], _timeout_secs: u64) -> Result<CommandOutput> {
        let key = Self::command_key(cmd, args);
        #[allow(clippy::unwrap_used)]
        let guard = self.run_responses.lock().unwrap();
        Ok(guard.get(&key).cloned().unwrap_or(CommandOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }))
    }

    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<i32> {
        let key = Self::command_key(cmd, args);
        #[allow(clippy::unwrap_used)]
        let guard = self.interactive_responses.lock().unwrap();
        Ok(guard.get(&key).copied().unwrap_or(0))
    }

    fn ssh_resolve(&self, host: &str, _config_args: &[String]) -> Result<String> {
        #[allow(clippy::unwrap_used)]
        let guard = self.ssh_resolve_responses.lock().unwrap();
        match guard.get(host) {
            Some(Ok(s)) => Ok(s.clone()),
            Some(Err(msg)) => anyhow::bail!("{msg}"),
            None => Ok(String::new()),
        }
    }

    fn ssh_agent_list(&self) -> Result<AgentListResult> {
        #[allow(clippy::unwrap_used)]
        let guard = self.agent_list_response.lock().unwrap();
        Ok(guard.clone().unwrap_or(AgentListResult {
            available: false,
            keys: Vec::new(),
        }))
    }

    fn ssh_keygen_fingerprint(&self, path: &Path) -> Result<String> {
        let key = path.to_string_lossy().into_owned();
        #[allow(clippy::unwrap_used)]
        let guard = self.fingerprint_responses.lock().unwrap();
        match guard.get(&key) {
            Some(Ok(s)) => Ok(s.clone()),
            Some(Err(msg)) => anyhow::bail!("{msg}"),
            None => Ok(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn mock_run_returns_registered_response() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "echo hello",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("hello\n"),
                stderr: String::new(),
            },
        );

        // Act
        let out = runner.run("echo", &["hello"], 5).unwrap();

        // Assert
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "hello\n");
    }

    #[test]
    fn mock_run_returns_default_for_unknown_command() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let out = runner.run("unknown", &[], 5).unwrap();

        // Assert
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn mock_ssh_resolve_returns_registered_output() {
        // Arrange
        let runner =
            MockCommandRunner::new().with_ssh_resolve("myhost", "hostname myhost\nport 22\n");

        // Act
        let result = runner.ssh_resolve("myhost", &[]).unwrap();

        // Assert
        assert!(result.contains("hostname myhost"));
    }

    #[test]
    fn mock_agent_list_returns_registered_result() {
        // Arrange
        let runner = MockCommandRunner::new().with_agent_list(AgentListResult {
            available: true,
            keys: vec![String::from("SHA256:abc123 user@host (RSA)")],
        });

        // Act
        let result = runner.ssh_agent_list().unwrap();

        // Assert
        assert!(result.available);
        assert_eq!(result.keys.len(), 1);
    }

    #[test]
    fn mock_fingerprint_returns_registered_value() {
        // Arrange
        let runner =
            MockCommandRunner::new().with_fingerprint("/home/user/.ssh/id_rsa", "SHA256:abc123");

        // Act
        let fp = runner
            .ssh_keygen_fingerprint(Path::new("/home/user/.ssh/id_rsa"))
            .unwrap();

        // Assert
        assert_eq!(fp, "SHA256:abc123");
    }

    #[test]
    fn mock_run_builds_key_correctly_with_args() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "git commit -m msg",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("committed"),
                stderr: String::new(),
            },
        );

        // Act
        let out = runner.run("git", &["commit", "-m", "msg"], 10).unwrap();

        // Assert
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "committed");
    }

    #[test]
    fn mock_ssh_resolve_returns_empty_for_unknown_host() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let result = runner.ssh_resolve("no-such-host", &[]).unwrap();

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn mock_agent_list_returns_unavailable_by_default() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let result = runner.ssh_agent_list().unwrap();

        // Assert
        assert!(!result.available);
        assert!(result.keys.is_empty());
    }

    #[test]
    fn mock_fingerprint_returns_empty_for_unknown_path() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let fp = runner
            .ssh_keygen_fingerprint(Path::new("/nonexistent/key"))
            .unwrap();

        // Assert
        assert!(fp.is_empty());
    }

    #[test]
    fn mock_run_returns_default_for_command_with_no_args() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "ls -la",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("files"),
                stderr: String::new(),
            },
        );

        // Act
        let out = runner.run("whoami", &[], 5).unwrap();

        // Assert
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty());
    }

    #[test]
    fn mock_ssh_resolve_error_returns_err() {
        // Arrange
        let runner =
            MockCommandRunner::new().with_ssh_resolve_error("badhost", "line 5: bad option");

        // Act
        let result = runner.ssh_resolve("badhost", &[]);

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("bad option"));
    }

    #[test]
    fn mock_fingerprint_error_returns_err() {
        // Arrange
        let runner = MockCommandRunner::new()
            .with_fingerprint_error("/home/user/.ssh/id_rsa", "invalid key format");

        // Act
        let result = runner.ssh_keygen_fingerprint(Path::new("/home/user/.ssh/id_rsa"));

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("invalid key format"));
    }

    #[test]
    fn mock_interactive_returns_registered_code() {
        // Arrange
        let runner = MockCommandRunner::new().with_interactive_response("tmux attach", 2);

        // Act
        let code = runner.run_interactive("tmux", &["attach"]).unwrap();

        // Assert
        assert_eq!(code, 2);
    }

    #[test]
    fn mock_interactive_returns_default_for_unknown() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let code = runner.run_interactive("unknown-cmd", &[]).unwrap();

        // Assert
        assert_eq!(code, 0);
    }
}
