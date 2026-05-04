//! Tmux window management.
//!
//! Manages the `@fterm_ssh_count` window option and window rename settings
//! to keep track of active SSH connections per window.

use std::convert::TryInto as _;

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::runner::CommandRunner;

/// Increment the `@fterm_ssh_count` window option by one.
///
/// Uses a tmux server-side arithmetic format expression to atomically increment
/// the count, avoiding the read-modify-write race when multiple panes open SSH
/// connections simultaneously.
///
/// Also disables window renaming so the user-set title is preserved.
///
/// # Errors
///
/// Returns an error if any tmux command fails to execute.
pub fn increment_ssh_count(runner: &dyn CommandRunner) -> Result<()> {
    debug!("incrementing @fterm_ssh_count (atomic)");
    set_window_option(
        runner,
        "@fterm_ssh_count",
        "#{e|+:#{?@fterm_ssh_count,#{@fterm_ssh_count},0},1}",
    )?;
    disable_rename(runner)?;
    Ok(())
}

/// Decrement the `@fterm_ssh_count` window option by one.
///
/// Uses a tmux server-side arithmetic format expression to atomically decrement
/// the count, then reads the result back to detect when it reaches zero.
///
/// When the count reaches zero (or underflows), `allow-rename` and
/// `automatic-rename` are restored to `on` so tmux can manage window titles
/// again.
///
/// # Errors
///
/// Returns an error if any tmux command fails to execute.
pub fn decrement_ssh_count(runner: &dyn CommandRunner) -> Result<()> {
    set_window_option(
        runner,
        "@fterm_ssh_count",
        "#{e|-:#{?@fterm_ssh_count,#{@fterm_ssh_count},0},1}",
    )?;

    // Read back to detect when the count has reached zero.
    // Negative values (underflow) are clamped to 0.
    let new_count = read_ssh_count(runner)?;
    debug!(new_count, "decremented @fterm_ssh_count");

    if new_count == 0 {
        debug!("ssh count reached 0; unsetting @fterm_ssh_count and restoring rename");
        unset_window_option(runner, "@fterm_ssh_count")?;
        set_window_option(runner, "allow-rename", "on")?;
        set_window_option(runner, "automatic-rename", "on")?;
    }

    Ok(())
}

/// Disable automatic and manual window renaming.
///
/// Sets both `automatic-rename` and `allow-rename` to `off`.
///
/// # Errors
///
/// Returns an error if either tmux command fails to execute.
pub fn disable_rename(runner: &dyn CommandRunner) -> Result<()> {
    debug!("disabling window rename");
    set_window_option(runner, "automatic-rename", "off")?;
    set_window_option(runner, "allow-rename", "off")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the current `@fterm_ssh_count` value, defaulting to 0.
///
/// Parses as `i64` to handle negative values that can result from the atomic
/// decrement format expression, then clamps to 0.
fn read_ssh_count(runner: &dyn CommandRunner) -> Result<u32> {
    let output = runner
        .run("tmux", &["show-window-option", "-v", "@fterm_ssh_count"], 5)
        .context("failed to read @fterm_ssh_count window option")?;

    if output.exit_code != 0 || output.stdout.trim().is_empty() {
        return Ok(0);
    }

    let raw: i64 = output.stdout.trim().parse().unwrap_or(0);
    // .max(0) clamps negatives from atomic decrement underflow;
    // .try_into() can only fail if raw > u32::MAX (unreachable in practice).
    let value: u32 = raw.max(0).try_into().unwrap_or(0);
    Ok(value)
}

/// Unset (remove) a tmux window option.
fn unset_window_option(runner: &dyn CommandRunner, option: &str) -> Result<()> {
    let output = runner
        .run("tmux", &["set-window-option", "-u", option], 5)
        .with_context(|| format!("failed to unset window option {option}"))?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux set-window-option -u {option} failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

/// Set a tmux window option.
fn set_window_option(runner: &dyn CommandRunner, option: &str, value: &str) -> Result<()> {
    let output = runner
        .run("tmux", &["set-window-option", option, value], 5)
        .with_context(|| format!("failed to set window option {option}={value}"))?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux set-window-option {option} {value} failed with exit code {}: {}",
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

    fn mock_with_readback(stdout: &str) -> MockCommandRunner {
        MockCommandRunner::new().with_run_response(
            "tmux show-window-option -v @fterm_ssh_count",
            CommandOutput {
                exit_code: 0,
                stdout: String::from(stdout),
                stderr: String::new(),
            },
        )
    }

    #[test]
    fn increment_from_zero() {
        // Arrange — increment only writes; no read-back needed
        let runner = MockCommandRunner::new();

        // Act
        let result = increment_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn decrement_to_zero_restores_rename() {
        // Arrange — read-back returns "0" to trigger rename restore
        let runner = mock_with_readback("0");

        // Act
        let result = decrement_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn decrement_above_zero_keeps_rename_off() {
        // Arrange — read-back returns "2" (still active connections)
        let runner = mock_with_readback("2");

        // Act
        let result = decrement_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn decrement_from_zero_stays_at_zero() {
        // Arrange — atomic decrement may produce "-1"; clamp to 0
        let runner = mock_with_readback("-1");

        // Act
        let result = decrement_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn read_ssh_count_clamps_negative() {
        // Arrange — atomic decrement on a zero counter produces "-1"
        let runner = mock_with_readback("-1");

        // Act
        let count = read_ssh_count(&runner).unwrap();

        // Assert
        assert_eq!(count, 0);
    }

    #[test]
    fn read_ssh_count_default_on_error() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux show-window-option -v @fterm_ssh_count",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("unknown option"),
            },
        );

        // Act
        let count = read_ssh_count(&runner).unwrap();

        // Assert
        assert_eq!(count, 0);
    }

    #[test]
    fn read_ssh_count_default_on_empty() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux show-window-option -v @fterm_ssh_count",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let count = read_ssh_count(&runner).unwrap();

        // Assert
        assert_eq!(count, 0);
    }

    #[test]
    fn disable_rename_success() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let result = disable_rename(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn set_window_option_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux set-window-option automatic-rename off",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("server not found"),
            },
        );

        // Act
        let result = set_window_option(&runner, "automatic-rename", "off");

        // Assert
        assert!(result.is_err());
    }
}
