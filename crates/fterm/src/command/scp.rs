//! SCP wrapper with validation and logging.
//!
//! Similar to the SSH wrapper but tailored for SCP file transfers:
//! validates each remote host, logs the session, and displays result banners.

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
use crate::util::scp_args::extract_user_host_pairs;
use crate::util::splash;
use crate::util::ssh_env;
use crate::validate::orchestrator::run_all_checks;

/// Run the SCP wrapper command.
///
/// Validates remote hosts, sets up logging and tmux integration, executes
/// `scp`, and displays a result banner.
///
/// # Errors
///
/// Returns an error if any internal operation (tmux, logging, validation)
/// fails unexpectedly.
#[tracing::instrument(skip(runner, args), err)]
#[allow(clippy::too_many_lines)]
pub fn run(runner: &dyn CommandRunner, args: &[String]) -> Result<i32> {
    let start_time = Instant::now();

    // Extract remote hosts (with optional explicit user from user@host:path form)
    let user_host_input = extract_user_host_pairs(args);
    let remote_hosts: Vec<String> = user_host_input.iter().map(|(_, h)| h.clone()).collect();
    if remote_hosts.is_empty() {
        debug!("no remote hosts found in args; exec scp directly");
        return Ok(exec_scp(args));
    }

    debug!(?remote_hosts, "extracted remote hosts from SCP args");

    if dry_run::is_scp(args) {
        debug!("dry-run flag detected; exec scp directly");
        return Ok(exec_scp(args));
    }

    // Pre-connect checks
    if let Some(code) =
        pre_connect_checks(runner, args, &remote_hosts).context("scp pre-connect checks failed")?
    {
        return Ok(code);
    }

    // Resolve all hosts individually via ssh -G
    let config_args: Vec<String> = build_config_args()?;
    let (user_host_pairs, first_ssh_g_output) =
        resolve_all_hosts(runner, &user_host_input, &config_args)?;

    // Build user@host display strings (used for log path, banners, and tmux state)
    let user_at_hosts: Vec<String> = user_host_pairs
        .iter()
        .map(|(u, h)| format!("{u}@{h}"))
        .collect();

    // Generate log path: scp_userA@hostA_userB@hostB.log
    let log_path = generate_scp_log_path(runner, &user_host_pairs);
    debug!(log_path = %log_path.display(), "generated SCP log path");

    // Get SSH details and agent keys from first host's resolve output
    let ssh_details = details::parse(&first_ssh_g_output);
    let agent_keys =
        crate::config::agent::get_matched_agent_keys_from_output(runner, &first_ssh_g_output)
            .unwrap_or_default();

    // Save original pane title for restore on teardown
    let original_pane_title = pane::get_title(runner).unwrap_or_default();

    // Setup (logging, banner, tmux)
    setup_scp_session(runner, &log_path, &user_at_hosts, &ssh_details, &agent_keys)?;

    // Execute SCP (directly, not via runner)
    let scp_exit_code = exec_scp_status(args, &config_args);
    let success = scp_exit_code == 0;

    // Teardown
    let elapsed = start_time.elapsed().as_secs();
    let duration_str = duration::format(elapsed);
    debug!(duration = %duration_str, exit_code = scp_exit_code, "SCP completed");

    teardown_scp_session(
        runner,
        &log_path,
        &user_at_hosts,
        success,
        &duration_str,
        &original_pane_title,
    );

    Ok(scp_exit_code)
}

/// Pre-connect checks: tmux, agent, and validation.
///
/// Returns `Some(exit_code)` for early exit, `None` to continue.
fn pre_connect_checks(
    runner: &dyn CommandRunner,
    args: &[String],
    remote_hosts: &[String],
) -> Result<Option<i32>> {
    // Tmux check
    if env::var("TMUX").is_err() {
        debug!("not inside tmux; delegating via ensure_tmux");
        let action =
            ensure_tmux(runner, "fterm", "scp", args).context("failed to ensure tmux session")?;
        if action == TmuxAction::DelegatedToTmux {
            return Ok(Some(0));
        }
    }

    // Load SSH_ENV file if set
    ssh_env::load();

    // SSH agent check
    let agent = runner
        .ssh_agent_list()
        .context("failed to check SSH agent")?;
    if !agent.available {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("Error: SSH agent is not available. Start ssh-agent first.");
        }
        return Ok(Some(1));
    }

    // Validation
    let ssh_home = get_dir();
    let config_path = ssh_home.join("config");
    let config_files = if config_path.exists() {
        resolve_included_files(&config_path, &ssh_home)
            .context("failed to resolve SSH config includes")?
    } else {
        Vec::new()
    };
    let config_args: Vec<String> = build_config_args()?;

    let validation = run_all_checks(runner, &ssh_home, &config_files, remote_hosts, &config_args)
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

/// Setup SCP session: logging, banner, and tmux state.
///
/// `user_at_hosts` is a slice of `"user@host"` strings, one per remote host.
fn setup_scp_session(
    runner: &dyn CommandRunner,
    log_path: &std::path::Path,
    user_at_hosts: &[String],
    ssh_details: &[String],
    agent_keys: &[String],
) -> Result<()> {
    // Derive display variants from user@host list
    let hosts_joined = user_at_hosts.join("_"); // for log header and pane title
    let hosts_display = user_at_hosts.join(" "); // for banner and @fterm_ssh_host

    // Start logging
    start::start(runner, log_path, &hosts_joined, ssh_details, agent_keys)
        .context("failed to start logging")?;

    // Print connect banner
    let banner = splash::scp_connect_banner(
        &hosts_display,
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

    // Set tmux pane title
    let pane_title = format!("scp:{hosts_joined}");
    if let Err(e) = pane::set_title(runner, &pane_title) {
        warn!("failed to set pane title: {e:#}");
    }

    // Increment SSH count (also disables rename)
    if let Err(e) = window::increment_ssh_count(runner) {
        warn!("failed to increment ssh count: {e:#}");
    }

    // Set @fterm_ssh_host (format: "scp:user1@host1 user2@host2")
    let scp_host_value = format!("scp:{hosts_display}");
    if let Err(e) = pane::set_ssh_host(runner, &scp_host_value) {
        warn!("failed to set @fterm_ssh_host: {e:#}");
    }

    Ok(())
}

/// Teardown SCP session: banner, tmux cleanup, logging.
fn teardown_scp_session(
    runner: &dyn CommandRunner,
    log_path: &std::path::Path,
    remote_hosts: &[String],
    success: bool,
    duration_str: &str,
    original_pane_title: &str,
) {
    // Print result banner
    let banner = splash::scp_result_banner(
        remote_hosts,
        success,
        duration_str,
        &log_path.to_string_lossy(),
    );
    #[allow(clippy::print_stderr)]
    {
        eprint!("{banner}");
    }

    // Reset pane style
    if let Err(e) = pane::reset_style(runner) {
        warn!("failed to reset pane style: {e:#}");
    }

    // Restore pane title
    if let Err(e) = pane::set_title(runner, original_pane_title) {
        warn!("failed to restore pane title: {e:#}");
    }

    // Unset @fterm_ssh_host
    if let Err(e) = pane::unset_ssh_host(runner) {
        warn!("failed to unset @fterm_ssh_host: {e:#}");
    }

    // Decrement SSH count (restores rename when reaching 0)
    if let Err(e) = window::decrement_ssh_count(runner) {
        warn!("failed to decrement ssh count: {e:#}");
    }

    // Stop logging
    if let Err(e) = stop::stop(runner, log_path) {
        warn!("failed to stop logging: {e:#}");
    }

    // Reset terminal title
    #[allow(clippy::print_stderr)]
    {
        eprint!("\x1b]0;\x07");
    }
}

/// Execute SCP directly using `std::process::Command::status()`.
///
/// Prepends `-F` config arguments so custom config dirs are honoured.
/// Returns the exit code of the SCP process.
fn exec_scp_status(args: &[String], config_args: &[String]) -> i32 {
    crate::external::exec_with_config("scp", args, config_args)
}

/// Execute SCP directly and return its exit code.
///
/// Used when no wrapping is needed (no remote hosts, dry-run).
/// Prepends `-F` config arguments when available.
fn exec_scp(args: &[String]) -> i32 {
    let config_args = build_config_args().unwrap_or_default();
    crate::external::exec_with_config("scp", args, &config_args)
}

/// Resolve SSH connection info for each remote host via `ssh -G`.
///
/// Returns a list of `(user, host)` pairs in the same order as `user_host_input`,
/// and the full `ssh -G` output for the first host (used for SSH details / agent keys).
///
/// If an explicit user was provided in the SCP argument (`user@host:path`),
/// that user is used as-is instead of querying `ssh -G`.
fn resolve_all_hosts(
    runner: &dyn CommandRunner,
    user_host_input: &[(Option<String>, String)],
    config_args: &[String],
) -> Result<(Vec<(String, String)>, String)> {
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(user_host_input.len());
    let mut first_output = String::new();

    for (i, (explicit_user, host)) in user_host_input.iter().enumerate() {
        let output = runner
            .ssh_resolve(host, config_args)
            .with_context(|| format!("failed to resolve host: {host}"))?;

        let user = explicit_user.as_ref().map_or_else(
            || {
                crate::config::connection::parse_connection_info(&output).map_or_else(
                    || {
                        warn!(host = %host, "could not parse connection info; defaulting to unknown user");
                        String::from("unknown")
                    },
                    |info| info.user,
                )
            },
            std::clone::Clone::clone,
        );

        if i == 0 {
            first_output = output;
        }
        pairs.push((user, host.clone()));
    }

    Ok((pairs, first_output))
}

/// Generate the log file path for SCP sessions.
///
/// Format: `{prefix}/{YYYY/MM/DD}/{YYYYMMDDTHHMMSS}_scp_{userA}@{hostA}_{userB}@{hostB}_{pane_pid}.log`
fn generate_scp_log_path(runner: &dyn CommandRunner, pairs: &[(String, String)]) -> PathBuf {
    let prefix = log_dir::get_prefix();
    let now = Local::now();
    let date_dir = now.format("%Y/%m/%d").to_string();
    let timestamp = now.format("%Y%m%dT%H%M%S").to_string();

    let pane_pid = get_pane_pid(runner);

    let hosts_part = pairs
        .iter()
        .map(|(user, host)| format!("{user}@{host}"))
        .collect::<Vec<_>>()
        .join("_");

    let filename = format!("{timestamp}_scp_{hosts_part}_{pane_pid}.log");

    PathBuf::from(&prefix).join(&date_dir).join(&filename)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;
    use crate::external::AgentListResult;
    use crate::external::CommandOutput;
    use fterm_core::runner::MockCommandRunner;

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
    fn resolve_all_hosts_uses_ssh_g_user() {
        // Arrange — two hosts, each with a distinct resolved user
        let runner = MockCommandRunner::new()
            .with_ssh_resolve("host1", "hostname 10.0.0.1\nuser alice\nport 22\n")
            .with_ssh_resolve("host2", "hostname 10.0.0.2\nuser bob\nport 22\n");
        let input = vec![(None, String::from("host1")), (None, String::from("host2"))];
        let config_args: Vec<String> = vec![];

        // Act
        let (pairs, first_output) = resolve_all_hosts(&runner, &input, &config_args).unwrap();

        // Assert
        assert_eq!(
            pairs,
            vec![
                (String::from("alice"), String::from("host1")),
                (String::from("bob"), String::from("host2")),
            ]
        );
        assert!(
            first_output.contains("alice"),
            "first_output should be host1 output"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn resolve_all_hosts_prefers_explicit_user() {
        // Arrange — explicit user in arg overrides ssh -G resolved user
        let runner = MockCommandRunner::new()
            .with_ssh_resolve("host1", "hostname 10.0.0.1\nuser config-user\nport 22\n");
        let input = vec![(Some(String::from("explicit")), String::from("host1"))];
        let config_args: Vec<String> = vec![];

        // Act
        let (pairs, _) = resolve_all_hosts(&runner, &input, &config_args).unwrap();

        // Assert — explicit user takes precedence
        assert_eq!(
            pairs,
            vec![(String::from("explicit"), String::from("host1"))]
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn resolve_all_hosts_defaults_unknown_on_parse_failure() {
        // Arrange — ssh -G returns empty output (parse fails)
        let runner = MockCommandRunner::new().with_ssh_resolve("host1", "");
        let input = vec![(None, String::from("host1"))];
        let config_args: Vec<String> = vec![];

        // Act
        let (pairs, _) = resolve_all_hosts(&runner, &input, &config_args).unwrap();

        // Assert — falls back to "unknown"
        assert_eq!(
            pairs,
            vec![(String::from("unknown"), String::from("host1"))]
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn generate_scp_log_path_contains_expected_parts() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("11111\n"),
                stderr: String::new(),
            },
        );
        let pairs = vec![
            (String::from("deploy"), String::from("host1")),
            (String::from("deploy"), String::from("host2")),
        ];

        // Act
        let path = generate_scp_log_path(&runner, &pairs);
        let path_str = path.to_string_lossy();

        // Assert — format: scp_{user@host}_{pane_pid}.log
        assert!(path_str.contains("scp_deploy@host1_deploy@host2_11111.log"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn generate_scp_log_path_single_host() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("22222\n"),
                stderr: String::new(),
            },
        );
        let pairs = vec![(String::from("root"), String::from("web-server"))];

        // Act
        let path = generate_scp_log_path(&runner, &pairs);
        let path_str = path.to_string_lossy();

        // Assert
        assert!(path_str.contains("scp_root@web-server_22222.log"));
        assert!(path_str.ends_with(".log"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn generate_scp_log_path_multi_host_different_users() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux display-message -p #{pane_pid}",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("33333\n"),
                stderr: String::new(),
            },
        );
        let pairs = vec![
            (String::from("alice"), String::from("host1")),
            (String::from("bob"), String::from("host2")),
        ];

        // Act
        let path = generate_scp_log_path(&runner, &pairs);
        let path_str = path.to_string_lossy();

        // Assert — each host gets its own user@host segment, followed by pane_pid
        assert!(path_str.contains("scp_alice@host1_bob@host2_33333.log"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn generate_scp_log_path_contains_date_directory() {
        // Arrange
        let runner = MockCommandRunner::new();
        let now = Local::now();
        let expected_date = now.format("%Y/%m/%d").to_string();
        let pairs = vec![(String::from("user"), String::from("host"))];

        // Act
        let path = generate_scp_log_path(&runner, &pairs);
        let path_str = path.to_string_lossy();

        // Assert
        assert!(
            path_str.contains(&expected_date),
            "path should contain date directory: {path_str}"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn generate_scp_log_path_contains_timestamp_prefix() {
        // Arrange
        let runner = MockCommandRunner::new();
        let now = Local::now();
        let expected_prefix = now.format("%Y%m%dT%H%M").to_string();
        let pairs = vec![(String::from("admin"), String::from("db-server"))];

        // Act
        let path = generate_scp_log_path(&runner, &pairs);
        let path_str = path.to_string_lossy();

        // Assert
        assert!(
            path_str.contains(&expected_prefix),
            "path should contain timestamp prefix: {path_str}"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn setup_scp_session_succeeds_with_mock_runner() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("scp_session.log");
        let runner = MockCommandRunner::new();
        let ssh_details = vec![String::from("hostname example.com")];
        let agent_keys = vec![String::from("SHA256:abc key@host (ED25519)")];
        let user_at_hosts = vec![String::from("alice@host1"), String::from("bob@host2")];

        // Act
        let result = setup_scp_session(
            &runner,
            &log_path,
            &user_at_hosts,
            &ssh_details,
            &agent_keys,
        );

        // Assert
        assert!(
            result.is_ok(),
            "setup_scp_session should succeed: {result:?}"
        );
        assert!(log_path.exists(), "log file should be created");
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("=== SSH Config ==="));
        assert!(content.contains("hostname example.com"));
        assert!(content.contains("=== Matched Agent Keys ==="));
        assert!(content.contains("SHA256:abc key@host (ED25519)"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn setup_scp_session_creates_log_directory() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("nested").join("dir").join("scp.log");
        let runner = MockCommandRunner::new();
        let user_at_hosts = vec![String::from("user@myhost")];

        // Act
        let result = setup_scp_session(&runner, &log_path, &user_at_hosts, &[], &[]);

        // Assert
        assert!(result.is_ok());
        assert!(log_path.exists());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn setup_scp_session_with_empty_details() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("empty.log");
        let runner = MockCommandRunner::new();
        let user_at_hosts = vec![String::from("user@host")];

        // Act
        let result = setup_scp_session(&runner, &log_path, &user_at_hosts, &[], &[]);

        // Assert
        assert!(result.is_ok());
        let content = std::fs::read_to_string(&log_path).unwrap();
        // No details/keys means no header at all
        assert_eq!(content, "");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn teardown_scp_session_does_not_panic() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("teardown.log");
        let runner = MockCommandRunner::new();
        let user_at_hosts = vec![String::from("alice@host1"), String::from("bob@host2")];

        // Act / Assert - should not panic regardless of success flag
        teardown_scp_session(&runner, &log_path, &user_at_hosts, true, "0s", "");
        teardown_scp_session(&runner, &log_path, &user_at_hosts, false, "0s", "");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn teardown_scp_session_single_host_success() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("single.log");
        let runner = MockCommandRunner::new();
        let user_at_hosts = vec![String::from("deploy@production")];

        // Act / Assert - should complete without panic
        teardown_scp_session(&runner, &log_path, &user_at_hosts, true, "0s", "");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn teardown_scp_session_failure_does_not_panic() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("fail.log");
        let runner = MockCommandRunner::new();
        let user_at_hosts = vec![String::from("ci@staging")];

        // Act / Assert - failure flag should not cause panic
        teardown_scp_session(&runner, &log_path, &user_at_hosts, false, "0s", "");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_agent_unavailable_returns_1() {
        // Arrange
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("TMUX", "/tmp/tmux-test/default,12345,0") };
        let runner = MockCommandRunner::new().with_agent_list(AgentListResult {
            available: false,
            keys: Vec::new(),
        });
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("host:~/"),
        ];
        let remote_hosts = vec![String::from("host")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("TMUX") };

        // Assert
        let code = result.unwrap();
        assert_eq!(
            code,
            Some(1),
            "should return Some(1) when agent is unavailable"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_agent_available_no_config_returns_none() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        // Create cm dir so cm_dir check passes
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        // Create identity file with 0600 permissions (avoids IdentityFile warning)
        let id_path = ssh_dir.join("id_test");
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();
        // No config file inside .ssh

        // Provide full ssh_resolve output so per-host checks pass
        let host_resolve = format!(
            "hostname 192.168.1.1\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile {id_path_str}\ncontrolpath /tmp/cm/%r@%h:%p\n"
        );

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
            // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
            // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
            env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap());
        };
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("host", &host_resolve);
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("host:~/"),
        ];
        let remote_hosts = vec![String::from("host")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            env::remove_var("FSSH_SSH_CONF_DIR");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert
        let code = result.unwrap();
        assert_eq!(
            code, None,
            "should return None when agent is available and no config exists"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_validation_errors_returns_1() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let config_path = ssh_dir.join("config");
        std::fs::write(&config_path, "Host errorhost\n  HostName 10.0.0.1\n").unwrap();

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
        };

        // Mock: agent available, syntax passes, but host resolve returns empty
        // output so basic checks produce errors (missing hostname, user, port).
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("errorhost", "");
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("errorhost:~/"),
        ];
        let remote_hosts = vec![String::from("errorhost")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — validation errors should cause Some(1)
        let code = result.unwrap();
        assert_eq!(
            code,
            Some(1),
            "should return Some(1) when validation has errors"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_validation_warnings_returns_none() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let config_path = ssh_dir.join("config");
        std::fs::write(&config_path, "Host warnhost\n  HostName 10.0.0.1\n").unwrap();
        // Create identity file with 0600 permissions (avoids IdentityFile warning)
        let id_path = ssh_dir.join("id_test");
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
            // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
            // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
            env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap());
        };

        // Mock: all required fields present, but identitiesonly=no triggers warning.
        let host_resolve = format!(
            "hostname 192.168.1.1\nuser deploy\nport 22\nidentitiesonly no\nidentityfile {id_path_str}\ncontrolpath /tmp/cm/%r@%h:%p\n"
        );
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("warnhost", &host_resolve);
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("warnhost:~/"),
        ];
        let remote_hosts = vec![String::from("warnhost")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            env::remove_var("FSSH_SSH_CONF_DIR");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — warnings only, should return None (continue)
        let code = result.unwrap();
        assert_eq!(
            code, None,
            "should return None when validation has only warnings"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn teardown_scp_session_handles_runner_errors() {
        // Arrange — register failing responses for tmux commands used in teardown
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("teardown_err.log");
        let runner = MockCommandRunner::new()
            .with_run_response(
                "tmux select-pane -P default",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no server"),
                },
            )
            .with_run_response(
                "tmux select-pane -T ",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no server"),
                },
            )
            .with_run_response(
                "tmux set-option -p -u @fterm_ssh_host",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no server"),
                },
            );
        let user_at_hosts = vec![String::from("user@host1")];

        // Act / Assert — should not panic despite all tmux commands failing
        teardown_scp_session(&runner, &log_path, &user_at_hosts, true, "0s", "");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn setup_scp_session_handles_pane_errors() {
        // Arrange — start::start needs a valid log path; pane/window cmds fail
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("setup_err.log");
        let user_at_hosts = vec![String::from("user@errhost")];
        let runner = MockCommandRunner::new()
            .with_run_response(
                "tmux select-pane -T scp:user@errhost",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no pane"),
                },
            )
            .with_run_response(
                "tmux set-window-option automatic-rename off",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no window"),
                },
            )
            .with_run_response(
                "tmux set-option -p @fterm_ssh_host scp:user@errhost",
                CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no pane"),
                },
            );

        // Act — setup should succeed (pane/window failures are just warnings)
        let result = setup_scp_session(&runner, &log_path, &user_at_hosts, &[], &[]);

        // Assert
        assert!(
            result.is_ok(),
            "setup should succeed even with pane errors: {result:?}"
        );
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn pre_connect_checks_not_in_tmux_delegates() {
        // Arrange — TMUX is unset; mock tmux commands to simulate delegation
        let original_tmux = env::var("TMUX").ok();
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::remove_var("TMUX") };

        // Mock ensure_tmux: tmux -V succeeds, has-session fails (no session),
        // new-session succeeds → DelegatedToTmux
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
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("host:~/"),
        ];
        let remote_hosts = vec![String::from("host")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts).unwrap();

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
    #[serial(env)]
    fn pre_connect_checks_with_valid_config_passes() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        // Create cm dir so cm_dir check passes
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let config_path = ssh_dir.join("config");
        std::fs::write(&config_path, "Host testhost\n  HostName 192.168.1.1\n").unwrap();
        // Create identity file with 0600 permissions (avoids IdentityFile warning)
        let id_path = ssh_dir.join("id_test");
        create_id_file_0600(&id_path);
        let id_path_str = id_path.to_str().unwrap();

        // Provide full ssh_resolve output so per-host checks pass
        let host_resolve = format!(
            "hostname 192.168.1.1\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile {id_path_str}\ncontrolpath /tmp/cm/%r@%h:%p\n"
        );

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
            // FSSH_SSH_CONF_DIR takes priority over HOME in get_dir(); this prevents
            // CI HOME mis-resolution from causing cm_dir Permission Denied errors.
            env::set_var("FSSH_SSH_CONF_DIR", ssh_dir.to_str().unwrap());
        };
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("testhost", &host_resolve);
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("testhost:~/"),
        ];
        let remote_hosts = vec![String::from("testhost")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            env::remove_var("FSSH_SSH_CONF_DIR");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert
        let code = result.unwrap();
        assert_eq!(code, None, "should return None when config is valid");
    }
}
