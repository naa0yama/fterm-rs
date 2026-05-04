//! Validation orchestrator — runs all checks and collects results.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::validate::{
    basic, cm_dir, control_path, duplicate, host_prefix, identity, proxyjump, syntax,
};
use fterm_core::check_types::{CheckLevel, CheckMessage, ValidationResult};
use fterm_core::runner::CommandRunner;

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// Run all validation checks against the SSH configuration.
///
/// # Errors
/// Returns an error if any critical I/O operation fails.
#[tracing::instrument(skip(runner, config_files, hosts, config_args), err)]
#[allow(clippy::too_many_lines)]
pub fn run_all_checks(
    runner: &dyn CommandRunner,
    ssh_home: &Path,
    config_files: &[PathBuf],
    hosts: &[String],
    config_args: &[String],
) -> Result<ValidationResult> {
    let mut messages: Vec<CheckMessage> = Vec::new();

    // 1. ControlMaster directory
    debug!("running cm_dir check");
    messages.extend(cm_dir::check(ssh_home));

    // 2. Syntax check — if it fails, return immediately
    debug!("running syntax check");
    let syntax_msgs = syntax::check(runner, config_args).context("syntax check failed")?;
    let has_syntax_errors = syntax_msgs.iter().any(|m| m.level == CheckLevel::Error);
    messages.extend(syntax_msgs);
    if has_syntax_errors {
        let (error_count, warn_count) = count_levels(&messages);
        return Ok(ValidationResult {
            messages,
            error_count,
            warn_count,
        });
    }

    // 3. Duplicate host detection
    debug!("running duplicate host check");
    let dup_msgs = duplicate::check(config_files).context("duplicate host check failed")?;
    messages.extend(dup_msgs);

    // 4. Per-host checks (resolve ssh -G once per host)
    for host in hosts {
        debug!(host = %host, "running per-host checks");

        // Host prefix
        match host_prefix::check(host, config_files) {
            Ok(msgs) => messages.extend(msgs),
            Err(e) => {
                warn!(host = %host, error = %e, "host_prefix check failed");
            }
        }

        // Resolve ssh -G output once for this host
        let ssh_g_output = match runner
            .ssh_resolve(host, config_args)
            .with_context(|| format!("failed to resolve host: {host}"))
        {
            Ok(output) => output,
            Err(e) => {
                warn!(host = %host, error = %e, "ssh_resolve failed; skipping per-host checks");
                continue;
            }
        };

        // Basic (pure function)
        messages.extend(basic::check(&ssh_g_output, host));

        // Identity
        match identity::check(runner, &ssh_g_output, host) {
            Ok(msgs) => messages.extend(msgs),
            Err(e) => {
                warn!(host = %host, error = %e, "identity check failed");
            }
        }

        // ProxyJump
        let mut visited = vec![host.clone()];
        match proxyjump::check(
            runner,
            &ssh_g_output,
            host,
            config_args,
            hosts,
            &mut visited,
        ) {
            Ok(msgs) => messages.extend(msgs),
            Err(e) => {
                warn!(host = %host, error = %e, "proxyjump check failed");
            }
        }

        // ControlPath (pure function)
        messages.extend(control_path::check(&ssh_g_output, host));
    }

    let (error_count, warn_count) = count_levels(&messages);
    Ok(ValidationResult {
        messages,
        error_count,
        warn_count,
    })
}

/// Count errors and warnings in a slice of messages.
fn count_levels(messages: &[CheckMessage]) -> (usize, usize) {
    let mut errors = 0usize;
    let mut warns = 0usize;
    for m in messages {
        match m.level {
            CheckLevel::Error => {
                errors = errors.saturating_add(1);
            }
            CheckLevel::Warn => {
                warns = warns.saturating_add(1);
            }
            CheckLevel::Info => {}
        }
    }
    (errors, warns)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use tempfile::TempDir;

    use fterm_core::check_types::{CheckLevel, CheckMessage};
    use fterm_core::runner::MockCommandRunner;

    use super::*;

    // -----------------------------------------------------------------------
    // run_all_checks tests
    // -----------------------------------------------------------------------

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_all_checks_pass_syntax_no_hosts_returns_empty_messages() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        // Create cm dir so cm_dir check passes
        std::fs::create_dir_all(ssh_home.join("conf.d").join("cm")).unwrap();

        let runner = MockCommandRunner::new().with_ssh_resolve(
            "syntax.check.dummy.host",
            "hostname syntax.check.dummy.host\n",
        );

        let config_files: Vec<PathBuf> = Vec::new();
        let hosts: Vec<String> = Vec::new();
        let config_args: Vec<String> = Vec::new();

        // Act
        let result =
            run_all_checks(&runner, ssh_home, &config_files, &hosts, &config_args).unwrap();

        // Assert — cm_dir passes (dir exists), syntax passes, duplicate passes (no files), no hosts
        assert_eq!(result.error_count, 0);
        assert_eq!(result.warn_count, 0);
        assert!(result.messages.is_empty());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_all_checks_syntax_fail_returns_early_with_syntax_errors() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        std::fs::create_dir_all(ssh_home.join("conf.d").join("cm")).unwrap();

        let runner = MockCommandRunner::new().with_ssh_resolve_error(
            "syntax.check.dummy.host",
            "/home/user/.ssh/config: line 5: Bad configuration option: xyz",
        );

        let config_files: Vec<PathBuf> = Vec::new();
        let hosts: Vec<String> = vec![String::from("myhost")];
        let config_args: Vec<String> = Vec::new();

        // Act
        let result =
            run_all_checks(&runner, ssh_home, &config_files, &hosts, &config_args).unwrap();

        // Assert — should have syntax errors and return early (no per-host checks)
        assert!(result.error_count > 0);
        assert!(result.messages.iter().any(|m| m.text.contains("[syntax]")));
        assert!(
            !result
                .messages
                .iter()
                .any(|m| m.text.contains("myhost") && !m.text.contains("[syntax]"))
        );
    }

    // -----------------------------------------------------------------------
    // count_levels tests
    // -----------------------------------------------------------------------

    #[test]
    fn count_levels_counts_correctly() {
        // Arrange
        let msgs = vec![
            CheckMessage {
                level: CheckLevel::Error,
                text: String::from("e1"),
            },
            CheckMessage {
                level: CheckLevel::Warn,
                text: String::from("w1"),
            },
            CheckMessage {
                level: CheckLevel::Error,
                text: String::from("e2"),
            },
        ];

        // Act
        let (errors, warns) = count_levels(&msgs);

        // Assert
        assert_eq!(errors, 2);
        assert_eq!(warns, 1);
    }

    #[test]
    fn count_levels_empty_messages_returns_zero() {
        // Arrange
        let msgs: Vec<CheckMessage> = Vec::new();

        // Act
        let (errors, warns) = count_levels(&msgs);

        // Assert
        assert_eq!(errors, 0);
        assert_eq!(warns, 0);
    }
}
