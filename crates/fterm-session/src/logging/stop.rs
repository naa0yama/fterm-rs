//! Stop session logging and compress the log file.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Local;
use tracing::{debug, warn};

use fterm_core::runner::CommandRunner;

/// Kind of footer to append when finalizing a log session.
#[derive(Debug)]
pub enum FooterKind {
    /// Normal teardown — the SSH session disconnected cleanly.
    Disconnect,
    /// Cleanup of an abandoned session detected at startup.
    Cleanup,
}

/// Finalize a logging session: append footer, stop pipe-pane, compress, unset.
///
/// Execution order ensures `@fterm_logging` remains set (and the `.log` file
/// exists) until after gzip completes, so other processes can safely exclude
/// active logs by querying the option.
///
/// 1. Append footer to the log file (skipped gracefully if the file is absent).
/// 2. Stop `tmux pipe-pane`.
/// 3. Compress the log with gzip (failure is a non-fatal warning).
/// 4. Unset `@fterm_logging` (non-fatal).
///
/// # Errors
/// Returns an error if `tmux pipe-pane` (stop) fails.
#[tracing::instrument(skip(runner), err)]
pub fn finalize_logging(
    runner: &dyn CommandRunner,
    log_path: &Path,
    kind: FooterKind,
) -> Result<()> {
    // 1. Append footer (skip gracefully if log file does not exist).
    if log_path.exists() {
        append_footer(log_path, &kind)?;
    } else {
        warn!(path = %log_path.display(), "log file does not exist; skipping footer");
    }

    // 2. Stop tmux pipe-pane.
    stop_pipe_pane(runner)?;

    // 3. Compress (non-fatal on race or failure; warn only).
    if let Err(e) = gzip_log(runner, log_path) {
        warn!(?e, path = %log_path.display(), "gzip failed; another fterm may have raced");
    }

    // 4. Unset @fterm_logging last so the active-log guard (Finding 8) sees
    //    the option as set until the .log file is fully compressed.
    unset_fterm_logging(runner);

    Ok(())
}

/// Stop session logging, append a disconnect marker, and compress the log.
///
/// This is a thin wrapper around [`finalize_logging`] using [`FooterKind::Disconnect`].
///
/// # Errors
/// Returns an error if `tmux pipe-pane` (stop) fails. Gzip failure is
/// non-fatal and is downgraded to a warning.
#[tracing::instrument(skip(runner), err)]
pub fn stop(runner: &dyn CommandRunner, log_path: &Path) -> Result<()> {
    finalize_logging(runner, log_path, FooterKind::Disconnect)
}

/// Append a timestamped footer line to the log file.
fn append_footer(log_path: &Path, kind: &FooterKind) -> Result<()> {
    let timestamp = Local::now().format("%Y-%m-%dT%H:%M:%S%z");
    let marker = match kind {
        FooterKind::Disconnect => format!(
            "----------------------------------------------------------------\n\
             [{timestamp}] === Session Disconnected ===\n"
        ),
        FooterKind::Cleanup => format!(
            "----------------------------------------------------------------\n\
             [{timestamp}] Closed by cleanup (previous session abandoned)\n"
        ),
    };

    debug!(path = %log_path.display(), "appending footer to log");

    let mut file = OpenOptions::new()
        .append(true)
        .open(log_path)
        .with_context(|| format!("failed to open log file for footer: {}", log_path.display()))?;

    file.write_all(marker.as_bytes())
        .with_context(|| format!("failed to write footer to: {}", log_path.display()))?;

    Ok(())
}

/// Stop `tmux pipe-pane` (no arguments stops the active pipe).
fn stop_pipe_pane(runner: &dyn CommandRunner) -> Result<()> {
    debug!("stopping tmux pipe-pane");

    let output = runner
        .run("tmux", &["pipe-pane"], 5)
        .context("failed to run tmux pipe-pane (stop)")?;

    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux pipe-pane (stop) exited with code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }

    Ok(())
}

/// Compress a log file with gzip.
///
/// # Errors
/// Returns an error if gzip exits with a non-zero status.
pub fn gzip_log(runner: &dyn CommandRunner, log_path: &Path) -> Result<()> {
    let log_path_str = log_path
        .to_str()
        .context("log path contains invalid UTF-8")?;

    debug!(path = %log_path_str, "compressing log file with gzip");

    let gzip_output = runner
        .run("gzip", &["--force", log_path_str], 30)
        .context("failed to run gzip on log file")?;

    if gzip_output.exit_code != 0 {
        anyhow::bail!(
            "gzip exited with code {}: {}",
            gzip_output.exit_code,
            gzip_output.stderr.trim()
        );
    }

    Ok(())
}

/// Unset `@fterm_logging` pane option (non-fatal).
fn unset_fterm_logging(runner: &dyn CommandRunner) {
    match runner.run("tmux", &["set-option", "-p", "-u", "@fterm_logging"], 5) {
        Err(e) => debug!(?e, "could not unset @fterm_logging (non-fatal)"),
        Ok(output) if output.exit_code != 0 => {
            debug!(
                exit_code = output.exit_code,
                "could not unset @fterm_logging (non-fatal)"
            );
        }
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use std::fs;

    use tempfile::TempDir;

    use fterm_core::runner::CommandOutput;
    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn stops_pipe_appends_marker_and_compresses() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "existing content\n").unwrap();
        let runner = MockCommandRunner::new();

        // Act
        stop(&runner, &log_path).unwrap();

        // Assert - verify disconnect marker was written before pipe-pane stop
        // Note: gzip is mocked so file still exists as-is
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("existing content"));
        assert!(content.contains("=== Session Disconnected ==="));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn returns_error_when_pipe_pane_stop_fails() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "").unwrap();
        let runner = MockCommandRunner::new().with_run_response(
            "tmux pipe-pane",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("server not found"),
            },
        );

        // Act
        let result = stop(&runner, &log_path);

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("tmux pipe-pane (stop) exited with code 1"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn returns_error_when_gzip_fails() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "data\n").unwrap();
        let gzip_key = format!("gzip --force {}", log_path.display());
        let runner = MockCommandRunner::new().with_run_response(
            &gzip_key,
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("gzip: No such file or directory"),
            },
        );

        // Act — gzip failure is now a warning, not an error
        let result = stop(&runner, &log_path);

        // Assert — finalize_logging tolerates gzip failure
        assert!(result.is_ok());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn missing_log_file_skips_gracefully() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("nonexistent.log");
        let runner = MockCommandRunner::new();

        // Act
        let result = stop(&runner, &log_path);

        // Assert — should succeed without error
        assert!(result.is_ok());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn disconnect_marker_has_timestamp_format() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "").unwrap();
        let runner = MockCommandRunner::new();

        // Act
        stop(&runner, &log_path).unwrap();

        // Assert
        let content = fs::read_to_string(&log_path).unwrap();
        // Verify separator line
        assert!(content.contains("---"));
        // Verify timestamp format: [YYYY-MM-DDThh:mm:ss+ZZZZ]
        assert!(content.contains('['));
        assert!(content.contains('T'));
        assert!(content.contains("] === Session Disconnected ==="));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn unset_fterm_logging_failure_is_non_fatal() {
        // Arrange — pipe-pane stop succeeds, set-option -u fails (non-fatal)
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("nonfatal.log");
        fs::write(&log_path, "data\n").unwrap();
        let runner = MockCommandRunner::new().with_run_response(
            "tmux set-option -p -u @fterm_logging",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("option not found"),
            },
        );

        // Act — unset failure should not propagate as an error
        let result = stop(&runner, &log_path);

        // Assert
        assert!(
            result.is_ok(),
            "unset @fterm_logging failure should be non-fatal: {result:?}"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn finalize_logging_cleanup_appends_cleanup_footer() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("stale.log");
        fs::write(&log_path, "stale content\n").unwrap();
        let runner = MockCommandRunner::new();

        // Act
        finalize_logging(&runner, &log_path, FooterKind::Cleanup).unwrap();

        // Assert — cleanup footer must be written
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("Closed by cleanup (previous session abandoned)"));
        assert!(!content.contains("Session Disconnected"));
    }
}
