#![allow(clippy::unwrap_used)]
#![allow(missing_docs)]

use assert_cmd::cargo_bin_cmd;
use predicates::prelude::predicate;

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("SSH/SCP connection management"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_version() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("fterm version"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_ssh_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.args(["ssh", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SSH wrapper"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_scp_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.args(["scp", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SCP wrapper"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_fssh_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.args(["fssh", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Interactive SSH host selection"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_flog_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.args(["flog", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Log viewer"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_fgen_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.args(["fgen", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SSH config template generator"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_log_filter_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.args(["log-filter", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ANSI filter"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cli_no_subcommand_shows_help() {
    let mut cmd = cargo_bin_cmd!("fterm");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}
