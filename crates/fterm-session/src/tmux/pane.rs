//! Tmux pane management.
//!
//! Utilities for setting pane titles, styles, and custom options used by fterm.

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::runner::CommandRunner;

/// Get the current pane title.
///
/// Runs `tmux display-message -p '#{pane_title}'`.
///
/// # Errors
///
/// Returns an error if the tmux command fails to execute.
pub fn get_title(runner: &dyn CommandRunner) -> Result<String> {
    let output = runner
        .run("tmux", &["display-message", "-p", "#{pane_title}"], 5)
        .context("failed to get tmux pane title")?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux display-message failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(output.stdout.trim().to_owned())
}

/// Set the title of the current tmux pane.
///
/// Runs `tmux select-pane -T <title>`.
///
/// # Errors
///
/// Returns an error if the tmux command fails to execute.
pub fn set_title(runner: &dyn CommandRunner, title: &str) -> Result<()> {
    debug!(title, "setting tmux pane title");
    let output = runner
        .run("tmux", &["select-pane", "-T", title], 5)
        .context("failed to set tmux pane title")?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux select-pane -T failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

/// Reset the style of the current tmux pane to the default.
///
/// Runs `tmux select-pane -P 'default'`.
///
/// # Errors
///
/// Returns an error if the tmux command fails to execute.
pub fn reset_style(runner: &dyn CommandRunner) -> Result<()> {
    debug!("resetting tmux pane style to default");
    let output = runner
        .run("tmux", &["select-pane", "-P", "default"], 5)
        .context("failed to reset tmux pane style")?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux select-pane -P failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

/// Set the `@fterm_ssh_host` pane option for the current pane.
///
/// Runs `tmux set-option -p @fterm_ssh_host <host>`.
///
/// # Errors
///
/// Returns an error if the tmux command fails to execute.
pub fn set_ssh_host(runner: &dyn CommandRunner, host: &str) -> Result<()> {
    debug!(host, "setting @fterm_ssh_host pane option");
    let output = runner
        .run("tmux", &["set-option", "-p", "@fterm_ssh_host", host], 5)
        .context("failed to set @fterm_ssh_host pane option")?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux set-option -p @fterm_ssh_host failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

/// Remove the `@fterm_ssh_host` pane option from the current pane.
///
/// Runs `tmux set-option -p -u @fterm_ssh_host`.
///
/// # Errors
///
/// Returns an error if the tmux command fails to execute.
pub fn unset_ssh_host(runner: &dyn CommandRunner) -> Result<()> {
    debug!("unsetting @fterm_ssh_host pane option");
    let output = runner
        .run("tmux", &["set-option", "-p", "-u", "@fterm_ssh_host"], 5)
        .context("failed to unset @fterm_ssh_host pane option")?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux set-option -p -u @fterm_ssh_host failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use fterm_core::runner::CommandOutput;
    use fterm_core::runner::MockCommandRunner;

    #[test]
    fn get_title_success() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_title}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("my-original-title\n"),
                stderr: String::new(),
            },
        );

        // Act
        let title = get_title(&runner).unwrap();

        // Assert
        assert_eq!(title, "my-original-title");
    }

    #[test]
    fn get_title_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_title}",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("no server"),
            },
        );

        // Act
        let result = get_title(&runner);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn set_title_success() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux select-pane -T my-host",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let result = set_title(&runner, "my-host");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn set_title_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux select-pane -T my-host",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("no current pane"),
            },
        );

        // Act
        let result = set_title(&runner, "my-host");

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("exit code 1"));
    }

    #[test]
    fn reset_style_success() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux select-pane -P default",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let result = reset_style(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn reset_style_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux select-pane -P default",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("error"),
            },
        );

        // Act
        let result = reset_style(&runner);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn set_ssh_host_success() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux set-option -p @fterm_ssh_host web-server",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let result = set_ssh_host(&runner, "web-server");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn set_ssh_host_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux set-option -p @fterm_ssh_host web-server",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("not in tmux"),
            },
        );

        // Act
        let result = set_ssh_host(&runner, "web-server");

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn unset_ssh_host_success() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux set-option -p -u @fterm_ssh_host",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let result = unset_ssh_host(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn unset_ssh_host_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux set-option -p -u @fterm_ssh_host",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("unknown option"),
            },
        );

        // Act
        let result = unset_ssh_host(&runner);

        // Assert
        assert!(result.is_err());
    }
}
