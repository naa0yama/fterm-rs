//! Start session logging via tmux pipe-pane.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::{debug, warn};

use super::stop::{FooterKind, finalize_logging};
use fterm_core::runner::CommandRunner;

/// Start session logging by writing a header and enabling tmux pipe-pane.
///
/// Creates the parent directory for `log_path` if it does not exist, writes
/// SSH config details and agent keys as a header block, then enables
/// `tmux pipe-pane` to continuously append filtered output to the log file.
///
/// # Errors
/// Returns an error if directory creation, file I/O, or the tmux command fails.
#[tracing::instrument(skip(runner, ssh_details, agent_keys), err)]
pub fn start(
    runner: &dyn CommandRunner,
    log_path: &Path,
    target_host: &str,
    ssh_details: &[String],
    agent_keys: &[String],
) -> Result<()> {
    // Detect and clean up a stale logging session left by a previous abnormal
    // exit (pane kill, fterm crash, SIGKILL). If @fterm_logging is still set,
    // the previous session never ran teardown; finalize it now.
    let prev_output = runner
        .run("tmux", &["show-option", "-pqv", "@fterm_logging"], 5)
        .context("failed to check @fterm_logging pane option")?;
    let prev_log = prev_output.stdout.trim();
    if !prev_log.is_empty() {
        warn!(
            previous = %prev_log,
            "stale logging session detected; running cleanup teardown"
        );
        let prev_path = std::path::PathBuf::from(prev_log);
        if let Err(e) = finalize_logging(runner, &prev_path, FooterKind::Cleanup) {
            warn!(?e, "cleanup teardown of stale session failed (continuing)");
        }
    }

    // Ensure the log directory exists.
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory: {}", parent.display()))?;
    }

    debug!(path = %log_path.display(), host = target_host, "writing log header");

    // Write header to log file.
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("failed to open log file: {}", log_path.display()))?;

    if !ssh_details.is_empty() {
        writeln!(file, "=== SSH Config ===")
            .with_context(|| format!("failed to write header to log: {}", log_path.display()))?;
        for detail in ssh_details {
            writeln!(file, "{detail}").with_context(|| {
                format!("failed to write SSH detail to log: {}", log_path.display())
            })?;
        }
    }
    if !agent_keys.is_empty() {
        writeln!(file, "=== Matched Agent Keys ===")
            .with_context(|| format!("failed to write header to log: {}", log_path.display()))?;
        for key in agent_keys {
            writeln!(file, "{key}").with_context(|| {
                format!("failed to write agent key to log: {}", log_path.display())
            })?;
        }
    }
    if !ssh_details.is_empty() || !agent_keys.is_empty() {
        writeln!(file)
            .with_context(|| format!("failed to write separator to log: {}", log_path.display()))?;
    }

    // Set up tmux pipe-pane with the log-filter command.
    let log_path_str = log_path
        .to_str()
        .context("log path contains invalid UTF-8")?;
    let escaped = log_path_str.replace('\'', "'\\''");
    let pipe_cmd = format!("exec fterm log-filter >> '{escaped}'");

    debug!(pipe_cmd = %pipe_cmd, "enabling tmux pipe-pane");

    let output = runner
        .run("tmux", &["pipe-pane", &pipe_cmd], 5)
        .context("failed to run tmux pipe-pane")?;

    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux pipe-pane exited with code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }

    // Set pane option to indicate logging is active.
    let set_output = runner
        .run(
            "tmux",
            &["set-option", "-p", "@fterm_logging", log_path_str],
            5,
        )
        .context("failed to set @fterm_logging pane option")?;
    if set_output.exit_code != 0 {
        debug!(
            exit_code = set_output.exit_code,
            "could not set @fterm_logging (non-fatal)"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use tempfile::TempDir;

    use fterm_core::runner::CommandOutput;
    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn creates_log_directory_and_writes_header() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("subdir").join("session.log");
        let runner = MockCommandRunner::new();
        let details = vec![
            String::from("hostname example.com"),
            String::from("port 22"),
        ];
        let keys = vec![String::from("SHA256:abc123 user@host (ED25519)")];

        // Act
        start(&runner, &log_path, "example.com", &details, &keys).unwrap();

        // Assert
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("=== SSH Config ==="));
        assert!(content.contains("hostname example.com"));
        assert!(content.contains("port 22"));
        assert!(content.contains("=== Matched Agent Keys ==="));
        assert!(content.contains("SHA256:abc123 user@host (ED25519)"));
        // Verify trailing blank line separator
        assert!(content.ends_with('\n'));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn returns_error_on_tmux_failure() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        let log_path_str = log_path.to_str().unwrap();
        let pipe_key = format!("tmux pipe-pane exec fterm log-filter >> '{log_path_str}'");
        let runner = MockCommandRunner::new().with_run_response(
            &pipe_key,
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("no server running"),
            },
        );
        let details = vec![];
        let keys = vec![];

        // Act
        let result = start(&runner, &log_path, "host", &details, &keys);

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("tmux pipe-pane exited with code 1"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn writes_empty_header_when_no_details() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        let runner = MockCommandRunner::new();

        // Act
        start(&runner, &log_path, "host", &[], &[]).unwrap();

        // Assert
        let content = fs::read_to_string(&log_path).unwrap();
        // No details/keys means no header at all
        assert_eq!(content, "");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn writes_header_with_details_only() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("details_only.log");
        let runner = MockCommandRunner::new();
        let details = vec![
            String::from("hostname web.example.com"),
            String::from("port 2222"),
        ];

        // Act
        start(&runner, &log_path, "web.example.com", &details, &[]).unwrap();

        // Assert
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("=== SSH Config ==="));
        assert!(content.contains("hostname web.example.com"));
        assert!(!content.contains("=== Matched Agent Keys ==="));
        assert!(content.ends_with('\n'));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn writes_header_with_keys_only() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("keys_only.log");
        let runner = MockCommandRunner::new();
        let keys = vec![String::from("SHA256:xyz user@machine (RSA)")];

        // Act
        start(&runner, &log_path, "host", &[], &keys).unwrap();

        // Assert
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(!content.contains("=== SSH Config ==="));
        assert!(content.contains("=== Matched Agent Keys ==="));
        assert!(content.contains("SHA256:xyz user@machine (RSA)"));
        assert!(content.ends_with('\n'));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn set_option_failure_is_non_fatal() {
        // Arrange — pipe-pane succeeds but set-option fails
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("nonfatal.log");
        let log_path_str = log_path.to_str().unwrap();
        let set_key = format!("tmux set-option -p @fterm_logging {log_path_str}");
        let runner = MockCommandRunner::new().with_run_response(
            &set_key,
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("no pane option"),
            },
        );

        // Act — set-option failure should not bubble up as an error
        let result = start(&runner, &log_path, "host", &[], &[]);

        // Assert
        assert!(
            result.is_ok(),
            "set-option failure should be non-fatal: {result:?}"
        );
    }
}
