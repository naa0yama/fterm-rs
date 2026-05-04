//! Identity file validation — existence, permissions, and agent presence.

#[cfg(unix)]
use std::path::Path;

use anyhow::{Context, Result};
use tracing::debug;

use fterm_core::check_types::{CheckLevel, CheckMessage};
use fterm_core::runner::CommandRunner;
use fterm_core::ssh_parse::parse_ssh_g_values;
use fterm_core::util::path::expand_tilde;

/// Validate identity files for a given host from pre-resolved `ssh -G` output.
///
/// For each `IdentityFile`:
/// 1. Check existence.
/// 2. Verify it is a valid key via `ssh-keygen -lf`.
/// 3. On Unix: warn if permissions are not `0o600`.
/// 4. Warn if the public key is not loaded in the SSH agent.
///
/// # Errors
/// Returns an error if the agent list or fingerprint commands fail.
pub fn check(
    runner: &dyn CommandRunner,
    ssh_g_output: &str,
    host: &str,
) -> Result<Vec<CheckMessage>> {
    let identity_files = parse_ssh_g_values(ssh_g_output, "identityfile");
    if identity_files.is_empty() {
        return Ok(Vec::new());
    }

    let agent = runner
        .ssh_agent_list()
        .context("identity check: failed to list agent keys")?;

    let mut messages = Vec::new();

    for raw_path in &identity_files {
        let path = expand_tilde(raw_path);

        // 1. Check existence
        if !path.exists() {
            messages.push(CheckMessage {
                level: CheckLevel::Error,
                text: format!("[{host}] IdentityFile not found: {}", path.display()),
            });
            continue;
        }

        // 2. Valid key check via ssh-keygen
        let fingerprint = match runner.ssh_keygen_fingerprint(&path) {
            Ok(fp) => fp,
            Err(e) => {
                messages.push(CheckMessage {
                    level: CheckLevel::Error,
                    text: format!(
                        "[{host}] IdentityFile is not a valid key ({}): {e}",
                        path.display()
                    ),
                });
                continue;
            }
        };

        // 3. Permission check (Unix only)
        #[cfg(unix)]
        {
            check_permissions(&path, host, &mut messages);
        }

        // 4. Agent check
        if agent.available && !fingerprint.is_empty() {
            let in_agent = agent.keys.iter().any(|k| {
                // The fingerprint line from ssh-keygen typically contains the
                // hash (e.g. SHA256:xxx). We look for that substring in agent
                // key lines.
                fingerprint
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|fp_hash| k.contains(fp_hash))
            });
            if !in_agent {
                // Public key files (.pub) without agent = certain auth failure
                let is_pub_key = std::path::Path::new(raw_path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pub"));
                let level = if is_pub_key {
                    CheckLevel::Error
                } else {
                    CheckLevel::Warn
                };
                messages.push(CheckMessage {
                    level,
                    text: format!(
                        "[{host}] IdentityFile public key not in agent: {}",
                        path.display()
                    ),
                });
            }
        }
    }

    debug!(host = %host, message_count = messages.len(), "identity check complete");
    Ok(messages)
}

/// Check file permissions on Unix, warning if not 0o600.
#[cfg(unix)]
fn check_permissions(path: &Path, host: &str, messages: &mut Vec<CheckMessage>) {
    use nix::sys::stat::stat;

    match stat(path) {
        Ok(file_stat) => {
            #[allow(clippy::as_conversions)]
            let mode = file_stat.st_mode & 0o777;
            if mode != 0o600 {
                messages.push(CheckMessage {
                    level: CheckLevel::Warn,
                    text: format!(
                        "[{host}] IdentityFile permissions are {mode:04o}, expected 0600: {}",
                        path.display()
                    ),
                });
            }
        }
        Err(e) => {
            messages.push(CheckMessage {
                level: CheckLevel::Warn,
                text: format!(
                    "[{host}] Could not stat IdentityFile ({}): {e}",
                    path.display()
                ),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::io::Write as _;

    use fterm_core::check_types::CheckLevel;
    use fterm_core::runner::AgentListResult;
    use fterm_core::runner::MockCommandRunner;

    use super::*;

    #[test]
    fn no_identity_files_returns_empty() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let msgs = check(&runner, "hostname example.com\n", "myhost").unwrap();

        // Assert
        assert!(msgs.is_empty());
    }

    #[test]
    fn missing_identity_file_returns_error() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let msgs = check(&runner, "identityfile /nonexistent/key\n", "myhost").unwrap();

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("not found"))
        );
    }

    #[test]
    fn existing_file_with_invalid_key_returns_error() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("id_test");
        {
            let mut f = std::fs::File::create(&key_path).unwrap();
            f.write_all(b"not a real key").unwrap();
        }

        let runner = MockCommandRunner::new()
            .with_fingerprint_error(&key_path.to_string_lossy(), "not a valid key");

        let output = format!("identityfile {}\n", key_path.display());

        // Act
        let msgs = check(&runner, &output, "myhost").unwrap();

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Error && m.text.contains("not a valid key"))
        );
    }

    #[cfg_attr(miri, ignore)] // check_permissions -> nix::sys::stat::stat (libc FFI), unsupported by Miri
    #[test]
    fn key_not_in_agent_returns_warn() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("id_test");
        {
            let mut f = std::fs::File::create(&key_path).unwrap();
            f.write_all(b"key content").unwrap();
        }

        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![String::from("2048 SHA256:otherkey user@host (RSA)")],
            })
            .with_fingerprint(
                &key_path.to_string_lossy(),
                "2048 SHA256:mykey user@host (RSA)",
            );

        let output = format!("identityfile {}\n", key_path.display());

        // Act
        let msgs = check(&runner, &output, "myhost").unwrap();

        // Assert
        assert!(
            msgs.iter()
                .any(|m| m.level == CheckLevel::Warn && m.text.contains("not in agent"))
        );
    }

    #[test]
    fn expand_tilde_works() {
        // Arrange / Act
        let expanded = expand_tilde("~/foo/bar");

        // Assert
        // Should not start with '~' after expansion (assuming HOME is set)
        if std::env::var("HOME").is_ok() {
            assert!(!expanded.starts_with("~"));
        }
    }
}
