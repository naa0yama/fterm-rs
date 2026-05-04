//! Syntax validation via `ssh -G`.

use anyhow::Result;
use tracing::debug;

use fterm_core::check_types::{CheckLevel, CheckMessage};
use fterm_core::runner::CommandRunner;

/// Run an SSH syntax check by resolving a dummy host.
///
/// Executes `ssh -G syntax.check.dummy.host` and inspects stderr for
/// error/bad/unknown/invalid keywords.
///
/// # Errors
/// Returns an error if the runner cannot execute the command.
pub fn check(runner: &dyn CommandRunner, config_args: &[String]) -> Result<Vec<CheckMessage>> {
    debug!("running ssh syntax check");

    let result = runner.ssh_resolve("syntax.check.dummy.host", config_args);

    match result {
        Ok(_) => {
            debug!("syntax check passed");
            Ok(Vec::new())
        }
        Err(e) => {
            let err_str = format!("{e:#}");
            debug!(error = %err_str, "syntax check failed, parsing error output");

            let mut messages = Vec::new();
            for line in err_str.lines() {
                let lower = line.to_lowercase();
                if lower.contains("error")
                    || lower.contains("bad")
                    || lower.contains("unknown")
                    || lower.contains("invalid")
                {
                    messages.push(CheckMessage {
                        level: CheckLevel::Error,
                        text: format!("[syntax] {}", line.trim()),
                    });
                }
            }

            // If no specific error lines matched, report the whole error.
            if messages.is_empty() {
                messages.push(CheckMessage {
                    level: CheckLevel::Error,
                    text: format!("[syntax] SSH config syntax error: {err_str}"),
                });
            }

            Ok(messages)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[test]
    fn syntax_ok_returns_empty() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve(
            "syntax.check.dummy.host",
            "hostname syntax.check.dummy.host\n",
        );

        // Act
        let msgs = check(&runner, &[]).unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn syntax_error_returns_error_messages() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve_error(
            "syntax.check.dummy.host",
            "/home/user/.ssh/config: line 5: Bad configuration option: xyz\nunknown keyword",
        );

        // Act
        let msgs = check(&runner, &[]).unwrap();

        // Assert
        assert!(!msgs.is_empty());
        assert!(msgs.iter().all(|m| m.level == CheckLevel::Error));
    }

    #[test]
    fn syntax_error_with_invalid_keyword() {
        // Arrange
        let runner = MockCommandRunner::new()
            .with_ssh_resolve_error("syntax.check.dummy.host", "line 10: invalid option");

        // Act
        let msgs = check(&runner, &[]).unwrap();

        // Assert
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].level == CheckLevel::Error);
        assert!(msgs[0].text.contains("invalid"));
    }

    #[test]
    fn syntax_error_with_multiple_lines() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve_error(
            "syntax.check.dummy.host",
            "line 3: Bad option xyz\nline 7: unknown keyword abc",
        );

        // Act
        let msgs = check(&runner, &[]).unwrap();

        // Assert
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|m| m.level == CheckLevel::Error));
    }

    #[test]
    fn syntax_error_with_mixed_lines() {
        // Arrange
        let runner = MockCommandRunner::new().with_ssh_resolve_error(
            "syntax.check.dummy.host",
            "some informational line\nline 5: error in config\nanother line",
        );

        // Act
        let msgs = check(&runner, &[]).unwrap();

        // Assert: only the line containing "error" should be captured
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].text.contains("error"));
    }

    #[test]
    fn syntax_error_no_keyword_match_still_reports() {
        // Arrange
        let runner = MockCommandRunner::new()
            .with_ssh_resolve_error("syntax.check.dummy.host", "something went wrong");

        // Act
        let msgs = check(&runner, &[]).unwrap();

        // Assert
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].text.contains("SSH config syntax error"));
    }
}
