//! Tmux session management.
//!
//! Handles ensuring the current process runs inside a tmux session,
//! creating or attaching to the `login-session` as needed.

use std::env;

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::runner::CommandRunner;

/// Fallback value when tmux identifiers cannot be retrieved.
const UNKNOWN_TMUX_ID: &str = "unknown-0.0";

/// Outcome of the [`ensure_tmux`] check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxAction {
    /// The process is already running inside tmux.
    AlreadyInTmux,
    /// The command was delegated to a tmux session via `send-keys`.
    DelegatedToTmux,
}

/// Ensure the current process runs inside a tmux session.
///
/// If `TMUX` is set the caller is already inside tmux and
/// [`TmuxAction::AlreadyInTmux`] is returned immediately.
///
/// Otherwise a `login-session` is created (or attached to) and the
/// reconstructed command (`{command_name} {subcommand} {command_args}`)
/// is sent via `tmux send-keys`.
///
/// # Errors
///
/// Returns an error if tmux is not installed, or if any tmux command fails.
pub fn ensure_tmux(
    runner: &dyn CommandRunner,
    command_name: &str,
    subcommand: &str,
    command_args: &[String],
) -> Result<TmuxAction> {
    // Already inside tmux — nothing to do.
    if env::var("TMUX").is_ok() {
        debug!("TMUX env var is set; already inside tmux");
        return Ok(TmuxAction::AlreadyInTmux);
    }

    // Verify tmux is installed by asking the runner to locate it.
    runner
        .run("tmux", &["-V"], 5)
        .context("tmux is not installed or not in PATH")?;

    // Build the escaped command string.
    let escaped_command = build_escaped_command(command_name, subcommand, command_args);
    debug!(escaped_command, "reconstructed command for send-keys");

    // Check whether the target session already exists.
    let has_session = runner
        .run("tmux", &["has-session", "-t", "login-session"], 5)
        .context("failed to check for existing tmux session")?;

    if has_session.exit_code != 0 {
        // Session does not exist — create it.
        debug!("creating new tmux session 'login-session'");
        runner
            .run("tmux", &["new-session", "-d", "-s", "login-session"], 5)
            .context("failed to create tmux session 'login-session'")?;
    }

    // Send the reconstructed command into the session.
    runner
        .run(
            "tmux",
            &[
                "send-keys",
                "-t",
                "login-session",
                &escaped_command,
                "Enter",
            ],
            5,
        )
        .context("failed to send keys to tmux session")?;

    // Attach to the session interactively so tmux can take over the terminal.
    let exit_code = runner
        .run_interactive("tmux", &["attach-session", "-t", "login-session"])
        .context("failed to attach to tmux session 'login-session'")?;

    if exit_code != 0 {
        anyhow::bail!("tmux attach-session exited with code {exit_code}");
    }

    Ok(TmuxAction::DelegatedToTmux)
}

/// Build a shell-safe command string from the command name, subcommand, and arguments.
///
/// Arguments containing whitespace or shell metacharacters are single-quoted.
/// An empty `subcommand` is omitted from the output.
fn build_escaped_command(command_name: &str, subcommand: &str, args: &[String]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(args.len().saturating_add(2));
    parts.push(shell_escape(command_name));
    if !subcommand.is_empty() {
        parts.push(shell_escape(subcommand));
    }
    for arg in args {
        parts.push(shell_escape(arg));
    }
    parts.join(" ")
}

/// Escape a single token for safe shell use.
///
/// If the token contains characters that need escaping it is wrapped in
/// single quotes with internal single quotes escaped.
fn shell_escape(token: &str) -> String {
    if token.is_empty() {
        return String::from("''");
    }
    // If the token is purely "safe" characters, return as-is.
    if token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | '@'))
    {
        return String::from(token);
    }
    // Wrap in single quotes, escaping embedded single quotes.
    let escaped = token.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Run `tmux display-message -p <format>` and return the trimmed stdout.
///
/// Returns `None` on command failure, non-zero exit, or empty output.
fn tmux_display_message(runner: &dyn CommandRunner, format: &str) -> Option<String> {
    let result = runner.run("tmux", &["display-message", "-p", format], 5);
    match result {
        Ok(output) if output.exit_code == 0 => {
            let trimmed = output.stdout.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(String::from(trimmed))
            }
        }
        _ => None,
    }
}

/// Get tmux session, window, and pane identifiers via `tmux display-message`.
///
/// Returns a string like `"main-2.1"` (session-window.pane).
/// Falls back to `"unknown-0.0"` on any failure.
#[must_use]
pub fn get_tmux_identifiers(runner: &dyn CommandRunner) -> String {
    tmux_display_message(runner, "#{session_name}-#{window_index}#{pane_index}")
        .unwrap_or_else(|| String::from(UNKNOWN_TMUX_ID))
}

/// Get the PID of the shell process running in the current tmux pane.
///
/// Uses `tmux display-message -p '#{pane_pid}'`.
/// Falls back to the current process ID when tmux is unavailable.
#[must_use]
pub fn get_pane_pid(runner: &dyn CommandRunner) -> String {
    tmux_display_message(runner, "#{pane_pid}").unwrap_or_else(|| std::process::id().to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serial_test::serial;

    use super::*;
    use fterm_core::runner::CommandOutput;
    use fterm_core::runner::MockCommandRunner;

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn already_in_tmux_returns_action() {
        // Arrange
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0") };
        let runner = MockCommandRunner::new();

        // Act
        let result = ensure_tmux(&runner, "fterm", "", &[]).unwrap();

        // Assert
        assert_eq!(result, TmuxAction::AlreadyInTmux);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("TMUX") };
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn delegates_when_session_exists() {
        // Arrange
        // SAFETY: test runs single-threaded; clearing env var for test isolation.
        unsafe { env::remove_var("TMUX") };
        let runner = MockCommandRunner::new()
            .with_run_response(
                "tmux -V",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::from("tmux 3.4"),
                    stderr: String::new(),
                },
            )
            .with_run_response(
                "tmux has-session -t login-session",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            )
            .with_run_response(
                "tmux send-keys -t login-session fterm ssh myhost Enter",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            )
            .with_interactive_response("tmux attach-session -t login-session", 0);

        // Act
        let result = ensure_tmux(&runner, "fterm", "ssh", &[String::from("myhost")]).unwrap();

        // Assert
        assert_eq!(result, TmuxAction::DelegatedToTmux);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn creates_session_when_missing() {
        // Arrange
        // SAFETY: test runs single-threaded; clearing env var for test isolation.
        unsafe { env::remove_var("TMUX") };
        let runner = MockCommandRunner::new()
            .with_run_response(
                "tmux -V",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::from("tmux 3.4"),
                    stderr: String::new(),
                },
            )
            .with_run_response(
                "tmux has-session -t login-session",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("can't find session: login-session"),
                },
            )
            .with_run_response(
                "tmux new-session -d -s login-session",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            )
            .with_run_response(
                "tmux send-keys -t login-session fterm Enter",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            )
            .with_interactive_response("tmux attach-session -t login-session", 0);

        // Act
        let result = ensure_tmux(&runner, "fterm", "", &[]).unwrap();

        // Assert
        assert_eq!(result, TmuxAction::DelegatedToTmux);
    }

    #[test]
    fn shell_escape_empty_string() {
        // Arrange / Act
        let result = shell_escape("");

        // Assert
        assert_eq!(result, "''");
    }

    #[test]
    fn shell_escape_simple_token() {
        // Arrange / Act
        let result = shell_escape("hello");

        // Assert
        assert_eq!(result, "hello");
    }

    #[test]
    fn shell_escape_token_with_spaces() {
        // Arrange / Act
        let result = shell_escape("hello world");

        // Assert
        assert_eq!(result, "'hello world'");
    }

    #[test]
    fn shell_escape_token_with_single_quotes() {
        // Arrange / Act
        let result = shell_escape("it's");

        // Assert
        assert_eq!(result, "'it'\\''s'");
    }

    #[test]
    fn build_escaped_command_basic() {
        // Arrange / Act
        let result = build_escaped_command("fterm", "ssh", &[String::from("host")]);

        // Assert
        assert_eq!(result, "fterm ssh host");
    }

    #[test]
    fn build_escaped_command_with_special_args() {
        // Arrange / Act
        let result = build_escaped_command("fterm", "ssh", &[String::from("my host")]);

        // Assert
        assert_eq!(result, "fterm ssh 'my host'");
    }

    #[test]
    fn build_escaped_command_empty_subcommand() {
        // Arrange / Act
        let result = build_escaped_command("fterm", "", &[]);

        // Assert
        assert_eq!(result, "fterm");
    }

    // -- get_tmux_identifiers tests --

    #[test]
    fn get_tmux_identifiers_returns_trimmed_stdout_on_success() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{session_name}-#{window_index}#{pane_index}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("main-2.1\n"),
                stderr: String::new(),
            },
        );

        // Act
        let result = get_tmux_identifiers(&runner);

        // Assert
        assert_eq!(result, "main-2.1");
    }

    #[test]
    fn get_tmux_identifiers_returns_fallback_on_empty_stdout() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{session_name}-#{window_index}#{pane_index}",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let result = get_tmux_identifiers(&runner);

        // Assert
        assert_eq!(result, "unknown-0.0");
    }

    #[test]
    fn get_tmux_identifiers_returns_fallback_on_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{session_name}-#{window_index}#{pane_index}",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("error: no tmux server"),
            },
        );

        // Act
        let result = get_tmux_identifiers(&runner);

        // Assert
        assert_eq!(result, "unknown-0.0");
    }

    // -- get_pane_pid tests --

    #[test]
    fn get_pane_pid_returns_trimmed_pid_on_success() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("8765\n"),
                stderr: String::new(),
            },
        );

        // Act
        let result = get_pane_pid(&runner);

        // Assert
        assert_eq!(result, "8765");
    }

    #[test]
    fn get_pane_pid_falls_back_to_process_id_on_empty_stdout() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let result = get_pane_pid(&runner);

        // Assert — fallback is a numeric process ID
        assert!(!result.is_empty(), "fallback should not be empty");
        assert!(
            result.parse::<u32>().is_ok(),
            "fallback should be a number: {result}"
        );
    }

    #[test]
    fn get_pane_pid_falls_back_to_process_id_on_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("error: no tmux server"),
            },
        );

        // Act
        let result = get_pane_pid(&runner);

        // Assert — fallback is a numeric process ID
        assert!(
            result.parse::<u32>().is_ok(),
            "fallback should be numeric: {result}"
        );
    }
}
