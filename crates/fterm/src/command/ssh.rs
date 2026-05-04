//! SSH wrapper with validation, logging, and tmux integration.
//!
//! Implements the 24-step SSH connection flow: validation, tmux setup,
//! logging, banner display, SSH execution, and cleanup.

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Local;
use fterm_core::check_types::format_summary;
use tracing::{debug, warn};

use crate::config::details;
use crate::config::home::{build_config_args, get_dir};
use crate::config::include::resolve_included_files;
use crate::external::CommandRunner;
use crate::logging::start;
use crate::logging::stop;
use crate::tmux::pane;
use crate::tmux::session::{TmuxAction, ensure_tmux, get_pane_pid};
use crate::tmux::window;
use crate::util::dry_run;
use crate::util::duration;
use crate::util::log_dir;
use crate::util::splash;
use crate::util::ssh_args::{extract_hostname_from_destination, extract_ssh_host};
use crate::util::ssh_env;
use crate::validate::orchestrator::run_all_checks;

/// Run the SSH wrapper command implementing the full connection flow.
///
/// Performs validation, tmux setup, logging, SSH execution, and cleanup.
/// Returns the exit code of the SSH process (or an early-exit code on
/// validation failure).
///
/// # Errors
///
/// Returns an error if any internal operation (tmux, logging, validation)
/// fails unexpectedly.
#[tracing::instrument(skip(runner, args), err)]
#[allow(clippy::too_many_lines)]
pub fn run(runner: &dyn CommandRunner, args: &[String]) -> Result<i32> {
    // Step 1: Record start time
    let start_time = Instant::now();

    // Steps 2-4: Extract host and check for early exits
    let Some(target_host) = extract_ssh_host(args) else {
        debug!("no target host found in args; exec ssh directly");
        return exec_ssh(args);
    };

    let hostname_only = extract_hostname_from_destination(&target_host);
    debug!(target_host = %target_host, hostname = %hostname_only, "parsed SSH destination");

    if dry_run::is_ssh(args) {
        debug!("dry-run flag detected; exec ssh directly");
        return exec_ssh(args);
    }

    // Steps 5-7: Tmux, agent, and validation pre-checks
    if let Some(code) =
        pre_connect_checks(runner, args, hostname_only).context("ssh pre-connect checks failed")?
    {
        return Ok(code);
    }

    // Step 8: Connection info (single ssh -G resolve)
    let config_args: Vec<String> = build_config_args()?;
    let ssh_g_output = runner
        .ssh_resolve(hostname_only, &config_args)
        .with_context(|| format!("failed to resolve host: {hostname_only}"))?;

    let conn =
        crate::config::connection::parse_connection_info(&ssh_g_output).unwrap_or_else(|| {
            warn!("could not resolve connection info for {hostname_only}; using defaults");
            crate::config::connection::Info {
                user: String::from("unknown"),
                hostname: String::from(hostname_only),
                port: String::from("22"),
            }
        });

    // Step 9: Generate log path
    let log_path = generate_log_path(runner, &conn.user, &conn.hostname, "ssh");
    debug!(log_path = %log_path.display(), "generated log path");

    // Get SSH details and agent keys from pre-resolved output
    let ssh_details = details::parse(&ssh_g_output);
    let agent_keys =
        crate::config::agent::get_matched_agent_keys_from_output(runner, &ssh_g_output)
            .unwrap_or_default();

    // Save original pane title for restore on teardown
    let original_pane_title = pane::get_title(runner).unwrap_or_default();

    // Steps 10-15: Setup (logging, banner, tmux state)
    setup_connection(
        runner,
        &log_path,
        &target_host,
        &conn,
        &ssh_details,
        &agent_keys,
    )
    .with_context(|| format!("failed to setup SSH connection to: {target_host}"))?;

    // Step 16: Execute SSH (directly, not via runner)
    let ssh_exit_code = exec_ssh_status(args, &config_args);

    // Steps 18-23: Teardown (banner, tmux state, logging)
    teardown_connection(runner, &log_path, &conn, start_time, &original_pane_title);

    // Step 24: Return SSH exit code
    Ok(ssh_exit_code)
}

/// Pre-connect checks: tmux, agent, validation (steps 5-7).
///
/// Returns `Some(exit_code)` if the caller should return early,
/// or `None` if the connection flow should continue.
fn pre_connect_checks(
    runner: &dyn CommandRunner,
    args: &[String],
    hostname_only: &str,
) -> Result<Option<i32>> {
    // Step 5: Tmux check
    if env::var("TMUX").is_err() {
        debug!("not inside tmux; delegating via ensure_tmux");
        let action =
            ensure_tmux(runner, "fterm", "ssh", args).context("failed to ensure tmux session")?;
        if action == TmuxAction::DelegatedToTmux {
            return Ok(Some(0));
        }
    }

    // Step 5.5: Load SSH_ENV file if set
    ssh_env::load();

    // Step 6: SSH agent check
    let has_identity_option = args.iter().any(|a| a == "-i");
    if !has_identity_option {
        let agent = runner
            .ssh_agent_list()
            .context("failed to check SSH agent")?;
        if !agent.available {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("Error: SSH agent is not available. Start ssh-agent or use -i option.");
            }
            return Ok(Some(1));
        }
    }

    // Step 7: Validation
    if let Some(code) = run_validation(runner, hostname_only)? {
        return Ok(Some(code));
    }

    Ok(None)
}

/// Run SSH config validation for the target host.
///
/// Returns `Some(1)` if validation errors were found, `None` otherwise.
fn run_validation(runner: &dyn CommandRunner, hostname_only: &str) -> Result<Option<i32>> {
    let ssh_home = get_dir();
    let config_path = ssh_home.join("config");
    let config_files = if config_path.exists() {
        resolve_included_files(&config_path, &ssh_home)
            .context("failed to resolve SSH config includes")?
    } else {
        Vec::new()
    };
    let target_hosts = vec![String::from(hostname_only)];
    let config_args: Vec<String> = build_config_args()?;

    let validation = run_all_checks(
        runner,
        &ssh_home,
        &config_files,
        &target_hosts,
        &config_args,
    )
    .context("SSH config validation failed")?;

    if validation.error_count > 0 {
        let summary = format_summary(&validation);
        #[allow(clippy::print_stderr)]
        {
            eprintln!("{summary}");
            for msg in &validation.messages {
                eprintln!("  {}", msg.text);
            }
        }
        return Ok(Some(1));
    }

    if validation.warn_count > 0 {
        let summary = format_summary(&validation);
        #[allow(clippy::print_stderr)]
        {
            eprintln!("{summary}");
        }
    }

    Ok(None)
}

/// Setup connection state: logging, banner, tmux options (steps 10-15).
fn setup_connection(
    runner: &dyn CommandRunner,
    log_path: &std::path::Path,
    target_host: &str,
    conn: &crate::config::connection::Info,
    ssh_details: &[String],
    agent_keys: &[String],
) -> Result<()> {
    // Step 10: Start logging
    start::start(runner, log_path, target_host, ssh_details, agent_keys)
        .context("failed to start logging")?;

    // Step 11: Print connect banner
    let banner = splash::ssh_connect_banner(
        target_host,
        &conn.user,
        &conn.hostname,
        &conn.port,
        &splash::BannerParams {
            log_path: &log_path.to_string_lossy(),
            ssh_details,
            agent_keys,
        },
    );
    #[allow(clippy::print_stderr)]
    {
        eprint!("{banner}");
    }

    // Step 12: Set tmux pane title
    let pane_title = format!("ssh:{}@{target_host}", conn.user);
    if let Err(e) = pane::set_title(runner, &pane_title) {
        warn!("failed to set pane title: {e:#}");
    }

    // Step 13: Increment SSH count
    if let Err(e) = window::increment_ssh_count(runner) {
        warn!("failed to increment ssh count: {e:#}");
    }

    // Step 14: Disable rename
    if let Err(e) = window::disable_rename(runner) {
        warn!("failed to disable rename: {e:#}");
    }

    // Step 15: Set @fterm_ssh_host (format: "ssh:user@host")
    let ssh_host_value = format!("ssh:{}@{target_host}", conn.user);
    if let Err(e) = pane::set_ssh_host(runner, &ssh_host_value) {
        warn!("failed to set @fterm_ssh_host: {e:#}");
    }

    Ok(())
}

/// Teardown after SSH session: banner, tmux cleanup, logging (steps 18-23).
fn teardown_connection(
    runner: &dyn CommandRunner,
    log_path: &std::path::Path,
    conn: &crate::config::connection::Info,
    start_time: Instant,
    original_pane_title: &str,
) {
    // Step 18: Calculate duration and print disconnect banner
    let elapsed = start_time.elapsed().as_secs();
    let duration_str = duration::format(elapsed);
    let disconnect_banner = splash::ssh_disconnect_banner(
        &conn.user,
        &conn.hostname,
        &duration_str,
        &log_path.to_string_lossy(),
    );
    #[allow(clippy::print_stderr)]
    {
        eprint!("{disconnect_banner}");
    }

    // Step 19: Reset pane style
    if let Err(e) = pane::reset_style(runner) {
        warn!("failed to reset pane style: {e:#}");
    }

    // Step 20: Restore pane title
    if let Err(e) = pane::set_title(runner, original_pane_title) {
        warn!("failed to restore pane title: {e:#}");
    }

    // Step 21: Unset @fterm_ssh_host
    if let Err(e) = pane::unset_ssh_host(runner) {
        warn!("failed to unset @fterm_ssh_host: {e:#}");
    }

    // Step 22: Decrement SSH count
    if let Err(e) = window::decrement_ssh_count(runner) {
        warn!("failed to decrement ssh count: {e:#}");
    }

    // Step 23: Stop logging
    if let Err(e) = stop::stop(runner, log_path) {
        warn!("failed to stop logging: {e:#}");
    }

    // Step 24: Reset terminal title
    #[allow(clippy::print_stderr)]
    {
        eprint!("\x1b]0;\x07");
    }
}

/// Execute SSH directly using `std::process::Command::status()`.
///
/// Used for interactive SSH sessions which may run for hours.
/// Prepends `-F` config arguments so custom config dirs are honoured.
/// Returns the exit code of the SSH process.
fn exec_ssh_status(args: &[String], config_args: &[String]) -> i32 {
    crate::external::exec_with_config("ssh", args, config_args)
}

/// Execute SSH directly, replacing the current process on Unix.
///
/// Used when no wrapping is needed (no host found, dry-run).
/// Prepends `-F` config arguments when available.
/// On Unix, uses `exec()` to replace the process (never returns on success).
/// On non-Unix, spawns a child process and returns the exit code.
///
/// # Errors
///
/// Returns an error if the SSH process cannot be spawned.
fn exec_ssh(args: &[String]) -> Result<i32> {
    let ssh_cmd = crate::external::resolve_ssh_command("ssh");
    let config_args = build_config_args().unwrap_or_default();
    let mut full_args: Vec<String> = Vec::new();
    for cfg in &config_args {
        full_args.push(String::from("-F"));
        full_args.push(cfg.clone());
    }
    full_args.extend_from_slice(args);
    let full_refs: Vec<&str> = full_args.iter().map(String::as_str).collect();

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&ssh_cmd).args(&full_refs).exec();
        // exec() only returns on error
        Err(err).context("failed to exec ssh")
    }

    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(&ssh_cmd)
            .args(&full_refs)
            .status()
            .context("failed to execute ssh")?;
        Ok(status.code().unwrap_or(1))
    }
}

/// Generate the log file path based on the current timestamp and tmux context.
///
/// Format: `{prefix}/{YYYY/MM/DD}/{YYYYMMDDTHHMMSS}_{session}-{window}{pane}_{cmd}_{user}@{hostname}.log`
fn generate_log_path(
    runner: &dyn CommandRunner,
    user: &str,
    hostname: &str,
    cmd_type: &str,
) -> PathBuf {
    let prefix = log_dir::get_prefix();
    let now = Local::now();
    let date_dir = now.format("%Y/%m/%d").to_string();
    let timestamp = now.format("%Y%m%dT%H%M%S").to_string();

    let pane_pid = get_pane_pid(runner);

    let filename = format!("{timestamp}_{cmd_type}_{user}@{hostname}_{pane_pid}.log");

    PathBuf::from(&prefix).join(&date_dir).join(&filename)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::undocumented_unsafe_blocks)]

    use serial_test::serial;

    use super::*;

    /// Create a file at `path` with `0600` permissions.
    /// Used in pre-connect-checks tests to satisfy `IdentityFile` validation.
    fn create_id_file_0600(path: &std::path::Path) {
        std::fs::write(path, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn generate_log_path_contains_expected_parts() {
        // Arrange
        use crate::external::CommandOutput;
        use fterm_core::runner::MockCommandRunner;
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("12345\n"),
                stderr: String::new(),
            },
        );

        // Act
        let path = generate_log_path(&runner, "deploy", "server1", "ssh");
        let path_str = path.to_string_lossy();

        // Assert
        assert!(path_str.contains("ssh_deploy@server1_12345.log"));
    }

    // -- parse_connection_info tests --

    #[test]
    fn parse_connection_info_with_valid_info() {
        // Arrange
        let output = "user deploy\nhostname 10.0.0.1\nport 2222\n";

        // Act
        let conn = crate::config::connection::parse_connection_info(output).unwrap();

        // Assert
        assert_eq!(conn.user, "deploy");
        assert_eq!(conn.hostname, "10.0.0.1");
        assert_eq!(conn.port, "2222");
    }

    #[test]
    fn parse_connection_info_returns_none_when_fields_missing() {
        // Arrange — empty output means no fields
        let conn = crate::config::connection::parse_connection_info("");

        // Assert
        assert!(conn.is_none());
    }

    // -- setup_connection tests --

    #[cfg_attr(miri, ignore)]
    #[test]
    fn setup_connection_runs_without_error() {
        // Arrange
        use fterm_core::runner::MockCommandRunner;
        let runner = MockCommandRunner::new();
        let log_path = PathBuf::from("/tmp/test.log");
        let conn = crate::config::connection::Info {
            user: String::from("deploy"),
            hostname: String::from("server1"),
            port: String::from("22"),
        };
        let ssh_details: Vec<String> = vec![String::from("identityfile ~/.ssh/id_ed25519")];
        let agent_keys: Vec<String> = vec![String::from("SHA256:abc123")];

        // Act
        let result = setup_connection(
            &runner,
            &log_path,
            "server1",
            &conn,
            &ssh_details,
            &agent_keys,
        );

        // Assert
        assert!(result.is_ok());
    }

    // -- teardown_connection tests --

    #[cfg_attr(miri, ignore)]
    #[test]
    fn teardown_connection_runs_without_error() {
        // Arrange
        use std::time::Instant;

        use fterm_core::runner::MockCommandRunner;
        let runner = MockCommandRunner::new();
        let log_path = PathBuf::from("/tmp/test.log");
        let conn = crate::config::connection::Info {
            user: String::from("deploy"),
            hostname: String::from("server1"),
            port: String::from("22"),
        };
        let start_time = Instant::now();

        // Act & Assert (no panic = success)
        teardown_connection(&runner, &log_path, &conn, start_time, "original-title");
    }

    // -- generate_log_path with tmux identifiers --

    // -- pre_connect_checks tests --

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn pre_connect_checks_agent_unavailable_returns_exit_code_1() {
        // Arrange
        use crate::external::AgentListResult;
        use fterm_core::runner::MockCommandRunner;

        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("TMUX", "test") };

        let runner = MockCommandRunner::new().with_agent_list(AgentListResult {
            available: false,
            keys: Vec::new(),
        });
        let args: Vec<String> = vec![String::from("server1")];

        // Act
        let result = pre_connect_checks(&runner, &args, "server1").unwrap();

        // Assert
        assert_eq!(result, Some(1));

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("TMUX") };
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_agent_available_passes() {
        // Arrange
        use tempfile::TempDir;

        use crate::external::AgentListResult;
        use fterm_core::runner::MockCommandRunner;

        // SAFETY: test runs single-threaded; env var is restored immediately.
        let original_home = env::var("HOME").ok();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_str().unwrap().to_owned();
        unsafe { env::set_var("TMUX", "test") };
        unsafe { env::set_var("HOME", &tmp_path) };

        // Create .ssh dir with cm subdirectory and identity file
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let id_path = ssh_dir.join("id_ed25519");
        // Create with 0600 permissions so IdentityFile check passes without warnings
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();

        // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
        // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
        unsafe { env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap()) };

        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: Vec::new(),
            })
            // Syntax check resolves this dummy host
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            // Per-host resolve for "server1" with all required fields
            .with_ssh_resolve("server1", &format!("hostname server1\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile {id_path_str}\n"));
        let args: Vec<String> = vec![String::from("server1")];

        // Act
        let result = pre_connect_checks(&runner, &args, "server1").unwrap();

        // Assert — no early exit, connection flow should continue
        assert_eq!(result, None);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("TMUX") };
        unsafe { env::remove_var("FSSH_SSH_CONF_DIR") };
        match original_home {
            Some(h) => unsafe { env::set_var("HOME", h) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_with_identity_option_skips_agent_check() {
        // Arrange
        use tempfile::TempDir;

        use crate::external::AgentListResult;
        use fterm_core::runner::MockCommandRunner;

        // SAFETY: test runs single-threaded; env var is restored immediately.
        let original_home = env::var("HOME").ok();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_str().unwrap().to_owned();
        unsafe { env::set_var("TMUX", "test") };
        unsafe { env::set_var("HOME", &tmp_path) };

        // Create .ssh dir with cm subdirectory and identity file
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let id_path = ssh_dir.join("id_ed25519");
        // Create with 0600 permissions so IdentityFile check passes without warnings
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();

        // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
        // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
        unsafe { env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap()) };

        // Agent is unavailable, but -i flag should skip agent check
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: false,
                keys: Vec::new(),
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("server1", &format!("hostname server1\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile {id_path_str}\n"));
        let args: Vec<String> = vec![
            String::from("-i"),
            String::from("/path/to/key"),
            String::from("server1"),
        ];

        // Act
        let result = pre_connect_checks(&runner, &args, "server1").unwrap();

        // Assert — should NOT return early despite agent being unavailable
        assert_eq!(result, None);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("TMUX") };
        unsafe { env::remove_var("FSSH_SSH_CONF_DIR") };
        match original_home {
            Some(h) => unsafe { env::set_var("HOME", h) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    // -- run_validation tests --

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_validation_no_config_returns_none() {
        // Arrange
        use tempfile::TempDir;

        use fterm_core::runner::MockCommandRunner;

        // SAFETY: test runs single-threaded; env var is restored immediately.
        let original_home = env::var("HOME").ok();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_str().unwrap().to_owned();
        unsafe { env::set_var("HOME", &tmp_path) };

        // Create .ssh dir but NO config file; also create cm dir to pass cm_dir check
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let id_path = ssh_dir.join("id_ed25519");
        // Create with 0600 permissions so IdentityFile check passes without warnings
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();

        // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
        // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
        unsafe { env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap()) };

        let runner = MockCommandRunner::new()
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("testhost", &format!("hostname testhost\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile {id_path_str}\n"));

        // Act
        let result = run_validation(&runner, "testhost").unwrap();

        // Assert — no config file means empty config_files, checks should pass
        assert_eq!(result, None);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("FSSH_SSH_CONF_DIR") };
        match original_home {
            Some(h) => unsafe { env::set_var("HOME", h) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_validation_with_valid_config_returns_none() {
        // Arrange
        use std::fs;

        use tempfile::TempDir;

        use fterm_core::runner::MockCommandRunner;

        // SAFETY: test runs single-threaded; env var is restored immediately.
        let original_home = env::var("HOME").ok();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_str().unwrap().to_owned();
        unsafe { env::set_var("HOME", &tmp_path) };

        // Create .ssh dir with config, cm dir, and identity file
        let ssh_dir = tmp.path().join(".ssh");
        fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let id_path = ssh_dir.join("id_ed25519");
        // Create with 0600 permissions so IdentityFile check passes without warnings
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();
        fs::write(
            ssh_dir.join("config"),
            format!(
                "Host myhost\n  HostName 10.0.0.1\n  User deploy\n  IdentityFile {id_path_str}\n"
            ),
        )
        .unwrap();

        // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
        // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
        unsafe { env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap()) };

        let runner = MockCommandRunner::new()
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve(
                "myhost",
                &format!("hostname 10.0.0.1\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile {id_path_str}\n"),
            );

        // Act
        let result = run_validation(&runner, "myhost").unwrap();

        // Assert — valid config should produce no errors
        assert_eq!(result, None);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("FSSH_SSH_CONF_DIR") };
        match original_home {
            Some(h) => unsafe { env::set_var("HOME", h) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_validation_with_errors_returns_some_1() {
        // Arrange
        use tempfile::TempDir;

        use fterm_core::runner::MockCommandRunner;

        let original_home = env::var("HOME").ok();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_str().unwrap().to_owned();
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("HOME", &tmp_path) };

        // Create .ssh dir with config and cm dir
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        std::fs::write(
            ssh_dir.join("config"),
            "Host errhost\n  HostName 10.0.0.1\n",
        )
        .unwrap();

        // Empty ssh_resolve output causes basic check errors (missing fields)
        let runner = MockCommandRunner::new()
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("errhost", "");

        // Act
        let result = run_validation(&runner, "errhost").unwrap();

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — errors should produce Some(1)
        assert_eq!(result, Some(1), "validation errors should return Some(1)");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_validation_with_warnings_returns_none() {
        // Arrange
        use tempfile::TempDir;

        use fterm_core::runner::MockCommandRunner;

        let original_home = env::var("HOME").ok();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_str().unwrap().to_owned();
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("HOME", &tmp_path) };

        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        std::fs::write(
            ssh_dir.join("config"),
            "Host warnhost\n  HostName 10.0.0.1\n",
        )
        .unwrap();
        // Create identity file with 0600 permissions (avoids IdentityFile warning)
        let id_path = ssh_dir.join("id_test");
        // Create with 0600 permissions so IdentityFile check passes without warnings
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();

        // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
        // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
        unsafe { env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap()) };

        // All required fields present, but identitiesonly=no triggers warning
        let host_resolve = format!(
            "hostname 192.168.1.1\nuser deploy\nport 22\nidentitiesonly no\nidentityfile {id_path_str}\ncontrolpath /tmp/cm/%r@%h:%p\n"
        );
        let runner = MockCommandRunner::new()
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("warnhost", &host_resolve);

        // Act
        let result = run_validation(&runner, "warnhost").unwrap();

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("FSSH_SSH_CONF_DIR");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — warnings only, should return None
        assert_eq!(
            result, None,
            "validation with only warnings should return None"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_validation_errors_returns_1() {
        // Arrange
        use tempfile::TempDir;

        use crate::external::AgentListResult;
        use fterm_core::runner::MockCommandRunner;

        let original_home = env::var("HOME").ok();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path().to_str().unwrap().to_owned();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "test");
            env::set_var("HOME", &tmp_path);
        };

        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        std::fs::write(
            ssh_dir.join("config"),
            "Host errhost\n  HostName 10.0.0.1\n",
        )
        .unwrap();

        // Empty resolve output causes validation errors
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: Vec::new(),
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("errhost", "");
        let args: Vec<String> = vec![String::from("errhost")];

        // Act
        let result = pre_connect_checks(&runner, &args, "errhost").unwrap();

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert
        assert_eq!(result, Some(1), "validation errors should produce Some(1)");
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    #[serial(env)]
    fn pre_connect_checks_not_in_tmux_delegates() {
        // Arrange — TMUX is unset; mock tmux commands to simulate delegation
        use crate::external::CommandOutput;
        use fterm_core::runner::MockCommandRunner;

        let original_tmux = env::var("TMUX").ok();
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::remove_var("TMUX") };

        // Mock ensure_tmux: tmux -V succeeds, has-session fails (no session),
        // new-session succeeds, send-keys succeeds → returns DelegatedToTmux
        let runner = MockCommandRunner::new()
            .with_run_response(
                "tmux -V",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::from("tmux 3.3a"),
                    stderr: String::new(),
                },
            )
            .with_run_response(
                "tmux has-session -t login-session",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            )
            .with_run_response(
                "tmux new-session -d -s login-session",
                CommandOutput {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            );
        let args: Vec<String> = vec![String::from("server1")];

        // Act
        let result = pre_connect_checks(&runner, &args, "server1").unwrap();

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            match &original_tmux {
                Some(v) => env::set_var("TMUX", v),
                None => env::remove_var("TMUX"),
            }
        };

        // Assert — delegated to tmux means early exit with Some(0)
        assert_eq!(result, Some(0));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn generate_log_path_uses_pane_pid() {
        // Arrange
        use crate::external::CommandOutput;
        use fterm_core::runner::MockCommandRunner;
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("9999\n"),
                stderr: String::new(),
            },
        );

        // Act
        let path = generate_log_path(&runner, "admin", "web01", "ssh");
        let path_str = path.to_string_lossy();

        // Assert — pane_pid is at the end, before .log
        assert!(path_str.contains("ssh_admin@web01_9999.log"));
    }
}
