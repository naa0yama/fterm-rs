//! External command execution — production implementations.
//!
//! The `CommandRunner` trait and mock live in `fterm-core`. This module
//! provides the production `RealCommandRunner` and helper functions that
//! call `std::process::Command` directly.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::debug;

pub use fterm_core::runner::{AgentListResult, CommandOutput, CommandRunner};

/// Hard timeout (seconds) for SSH-family helper commands.
///
/// Matches fish `__fterm_run_ssh_cmd` behaviour: gpg-agent / ssh-agent
/// forwarding can freeze indefinitely, which would block the terminal.
/// Every helper invocation is killed after this many seconds.
const SSH_HELPER_TIMEOUT_SECS: u64 = 1;

/// Resolve the path for an SSH-related command.
///
/// On MSYS2, searches known Windows OpenSSH directories for `{name}.exe`.
/// Otherwise returns the bare command name for PATH lookup.
pub(crate) fn resolve_ssh_command(name: &str) -> String {
    crate::util::path::resolve_win_ssh_command(name).unwrap_or_else(|| String::from(name))
}

/// Execute an SSH-family command with `-F` config arguments prepended.
///
/// Builds the full argument list (`-F cfg1 -F cfg2 … user_args…`), runs the
/// command via `std::process::Command::status()`, and returns the exit code.
/// Used for interactive sessions (SSH, SCP) that may run for a long time.
#[tracing::instrument(skip(config_args))]
pub fn exec_with_config(command_name: &str, args: &[String], config_args: &[String]) -> i32 {
    let cmd = resolve_ssh_command(command_name);
    let mut full_args: Vec<&str> = Vec::new();
    for cfg in config_args {
        full_args.push("-F");
        full_args.push(cfg.as_str());
    }
    for a in args {
        full_args.push(a.as_str());
    }
    let mut command = std::process::Command::new(&cmd);
    command.args(&full_args);
    // MSYS2: set HOME to Windows mixed path so Include resolves correctly
    if let Some(home) = crate::util::path::msys2_home() {
        command.env("HOME", &home);
    }
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            tracing::error!("failed to execute {command_name}: {e:#}");
            1
        }
    }
}

/// Execute a command as a passthrough, replacing the process on Unix.
///
/// On Unix, uses `exec()` to replace the current process (never returns on
/// success). On non-Unix, spawns a child process and returns the exit code.
///
/// # Errors
///
/// Returns an error if the command cannot be spawned.
// NOTEST(ffi): Unix exec() replaces the process; success path never returns
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn exec_passthrough(cmd: &str, args: &[&str]) -> Result<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let err = Command::new(cmd).args(args).exec();
        // exec() only returns on error
        Err(err).with_context(|| format!("failed to exec {cmd}"))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new(cmd)
            .args(args)
            .status()
            .with_context(|| format!("failed to execute {cmd}"))?;
        Ok(status.code().unwrap_or(1))
    }
}

// ---------------------------------------------------------------------------
// RealCommandRunner
// ---------------------------------------------------------------------------

/// Production implementation that executes real OS commands.
#[derive(Debug, Default)]
pub struct RealCommandRunner;

impl RealCommandRunner {
    /// Create a new `RealCommandRunner`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CommandRunner for RealCommandRunner {
    /// Run an external command, capturing stdout and stderr.
    ///
    /// Non-zero exit codes are **not** treated as errors – the caller decides
    /// how to interpret the exit code via [`CommandOutput`].
    ///
    /// # Errors
    /// Returns an error only when the command cannot be spawned.
    #[tracing::instrument(skip(self), err)]
    fn run(&self, cmd: &str, args: &[&str], timeout_secs: u64) -> Result<CommandOutput> {
        debug!(
            command = cmd,
            ?args,
            timeout_secs,
            "spawning external command"
        );

        if timeout_secs > 0 {
            let mut child = Command::new(cmd)
                .args(args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| format!("failed to spawn command: {cmd}"))?;

            #[allow(clippy::arithmetic_side_effects)]
            let deadline = Instant::now() + Duration::from_secs(timeout_secs);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let stdout_bytes = child.stdout.take().map_or_else(Vec::new, |mut r| {
                            let mut buf = Vec::new();
                            std::io::Read::read_to_end(&mut r, &mut buf).unwrap_or(0);
                            buf
                        });
                        let stderr_bytes = child.stderr.take().map_or_else(Vec::new, |mut r| {
                            let mut buf = Vec::new();
                            std::io::Read::read_to_end(&mut r, &mut buf).unwrap_or(0);
                            buf
                        });
                        let stdout = String::from_utf8_lossy(&stdout_bytes).replace('\r', "");
                        let stderr = String::from_utf8_lossy(&stderr_bytes).replace('\r', "");
                        let exit_code = status.code().unwrap_or(-1);
                        debug!(command = cmd, exit_code, "command finished");
                        return Ok(CommandOutput {
                            exit_code,
                            stdout,
                            stderr,
                        });
                    }
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            anyhow::bail!("command timed out after {timeout_secs}s: {cmd}");
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        return Err(e)
                            .with_context(|| format!("failed to wait for command: {cmd}"));
                    }
                }
            }
        }

        let output = Command::new(cmd)
            .args(args)
            .output()
            .with_context(|| format!("failed to spawn command: {cmd}"))?;

        // Strip \r for Windows compatibility.
        let stdout = String::from_utf8_lossy(&output.stdout).replace('\r', "");
        let stderr = String::from_utf8_lossy(&output.stderr).replace('\r', "");

        let exit_code = output.status.code().unwrap_or(-1);
        debug!(command = cmd, exit_code, "command finished");

        Ok(CommandOutput {
            exit_code,
            stdout,
            stderr,
        })
    }

    /// Run a command interactively with inherited stdio.
    ///
    /// # Errors
    /// Returns an error if the command cannot be spawned.
    #[tracing::instrument(skip(self), err)]
    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<i32> {
        debug!(command = cmd, ?args, "spawning interactive command");

        let status = Command::new(cmd)
            .args(args)
            .status()
            .with_context(|| format!("failed to spawn interactive command: {cmd}"))?;

        let exit_code = status.code().unwrap_or(-1);
        debug!(command = cmd, exit_code, "interactive command finished");

        Ok(exit_code)
    }

    /// Resolve SSH configuration for `host` by running `ssh -G`.
    ///
    /// # Errors
    /// Returns an error if `ssh -G` cannot be spawned or exits with non-zero.
    #[tracing::instrument(skip(self, config_args), err)]
    fn ssh_resolve(&self, host: &str, config_args: &[String]) -> Result<String> {
        let mut args: Vec<&str> = Vec::new();
        for arg in config_args {
            args.push("-F");
            args.push(arg.as_str());
        }
        args.push("-G");
        args.push(host);

        let ssh_cmd = resolve_ssh_command("ssh");
        let result = self
            .run(&ssh_cmd, &args, SSH_HELPER_TIMEOUT_SECS)
            .with_context(|| format!("ssh_resolve failed for host: {host}"))?;

        if result.exit_code != 0 {
            anyhow::bail!(
                "ssh -G {host} exited with code {}: {}",
                result.exit_code,
                result.stderr.trim()
            );
        }

        Ok(result.stdout)
    }

    /// List keys held by the SSH agent.
    ///
    /// # Errors
    /// Returns an error if `ssh-add` cannot be spawned.
    fn ssh_agent_list(&self) -> Result<AgentListResult> {
        let ssh_add_cmd = resolve_ssh_command("ssh-add");
        let result = self
            .run(&ssh_add_cmd, &["-l"], SSH_HELPER_TIMEOUT_SECS)
            .context("failed to list SSH agent keys")?;

        if result.exit_code == 0 {
            let keys = result
                .stdout
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            Ok(AgentListResult {
                available: true,
                keys,
            })
        } else {
            Ok(AgentListResult {
                available: false,
                keys: Vec::new(),
            })
        }
    }

    /// Obtain the fingerprint of an SSH key file.
    ///
    /// # Errors
    /// Returns an error if `ssh-keygen` cannot be spawned or fails.
    fn ssh_keygen_fingerprint(&self, path: &Path) -> Result<String> {
        let path_str = path.to_str().context("key path contains invalid UTF-8")?;

        let ssh_keygen_cmd = resolve_ssh_command("ssh-keygen");
        let result = self
            .run(&ssh_keygen_cmd, &["-lf", path_str], SSH_HELPER_TIMEOUT_SECS)
            .with_context(|| format!("ssh_keygen_fingerprint failed for: {}", path.display()))?;

        if result.exit_code != 0 {
            anyhow::bail!(
                "ssh-keygen -lf {} exited with code {}: {}",
                path.display(),
                result.exit_code,
                result.stderr.trim()
            );
        }

        Ok(result.stdout.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn real_runner_new_creates_instance() {
        // Arrange & Act
        let runner = RealCommandRunner::new();

        // Assert
        let debug_str = format!("{runner:?}");
        assert_eq!(debug_str, "RealCommandRunner");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_returns_success_for_true() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("true", &[], 5).unwrap();

        // Assert
        assert_eq!(result.exit_code, 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_returns_nonzero_for_false() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("false", &[], 5).unwrap();

        // Assert
        assert_ne!(result.exit_code, 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_captures_stdout() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("echo", &["hello"], 5).unwrap();

        // Assert
        assert_eq!(result.stdout, "hello\n");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_returns_error_for_nonexistent_command() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("this_command_does_not_exist_xyz", &[], 5);

        // Assert
        assert!(result.is_err());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_interactive_returns_success_for_true() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let exit_code = runner.run_interactive("true", &[]).unwrap();

        // Assert
        assert_eq!(exit_code, 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_interactive_returns_nonzero_for_false() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let exit_code = runner.run_interactive("false", &[]).unwrap();

        // Assert
        assert_ne!(exit_code, 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_ssh_agent_list_returns_result() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.ssh_agent_list();

        // Assert
        assert!(result.is_ok());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_ssh_keygen_fingerprint_returns_error_for_nonexistent_file() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.ssh_keygen_fingerprint(Path::new("/nonexistent/key"));

        // Assert
        assert!(result.is_err());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_ssh_resolve_returns_result_for_localhost() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let _result = runner.ssh_resolve("localhost", &[]);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_with_timeout_captures_stdout() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("echo", &["hello-timeout"], 5).unwrap();

        // Assert
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello-timeout\n");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_with_timeout_nonzero_exit() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("false", &[], 5).unwrap();

        // Assert
        assert_ne!(result.exit_code, 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_with_timeout_spawn_error() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("this_command_does_not_exist_timeout_xyz", &[], 5);

        // Assert
        assert!(result.is_err());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn real_runner_run_timeout_kills_long_process() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("sleep", &["60"], 1);

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("timed out") || err_msg.contains("timeout"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn exec_with_config_returns_zero_for_true() {
        // Arrange
        let args: Vec<String> = vec![];
        let config_args: Vec<String> = vec![];

        // Act
        let code = exec_with_config("true", &args, &config_args);

        // Assert
        assert_eq!(code, 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn exec_with_config_returns_nonzero_for_false() {
        // Arrange
        let args: Vec<String> = vec![];
        let config_args: Vec<String> = vec![];

        // Act
        let code = exec_with_config("false", &args, &config_args);

        // Assert
        assert_ne!(code, 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn exec_with_config_returns_1_for_nonexistent_command() {
        // Arrange
        let args: Vec<String> = vec![];
        let config_args: Vec<String> = vec![];

        // Act
        let code = exec_with_config("__nonexistent_command_xyz__", &args, &config_args);

        // Assert
        assert_eq!(code, 1);
    }
}
