//! ssh-add passthrough wrapper.
//!
//! On MSYS2, routes to the Windows OpenSSH `ssh-add.exe`.
//! On Unix, replaces the current process via `exec()`.

use anyhow::Result;
use tracing::debug;

use crate::external::{exec_passthrough, resolve_ssh_command};

/// Run ssh-add with the given arguments.
///
/// On Unix, this replaces the current process (never returns on success).
/// On non-Unix, spawns a child process and returns the exit code.
///
/// # Errors
///
/// Returns an error if the command cannot be spawned.
pub fn run(args: &[String]) -> Result<i32> {
    let cmd = resolve_ssh_command("ssh-add");
    let args_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    debug!(command = %cmd, ?args_refs, "executing ssh-add");

    exec_passthrough(&cmd, &args_refs)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_with_nonexistent_command_returns_error() {
        // Arrange — use an invalid command name to trigger spawn failure
        // We cannot easily test exec() on Unix since it replaces the process,
        // but we can verify error handling by testing resolve_ssh_command indirectly.
        let result = exec_passthrough("this_command_does_not_exist_xyz_ssh_add", &["-l"]);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn resolve_ssh_command_returns_ssh_add() {
        // Arrange / Act
        let cmd = resolve_ssh_command("ssh-add");

        // Assert — on non-MSYS2 it should return the bare name
        assert!(
            cmd.contains("ssh-add"),
            "resolved command should contain ssh-add: {cmd}"
        );
    }
}
