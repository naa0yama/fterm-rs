//! Shell completion and integration script generation.
//!
//! Generates shell completion scripts for bash and fish, including wrapper
//! functions for interactive commands like `fssh`.

use std::io;

use anyhow::{Context, Result};
use clap::CommandFactory;
use clap_complete::shells;

use crate::cli::{Cli, ShellType};
use crate::config::home::get_dir;
use crate::config::hosts;
use crate::config::include::resolve_included_files;

/// Generate and print shell completions to stdout.
///
/// When `list_hosts` is true, prints SSH host names (one per line) for use
/// by completion scripts. Otherwise, prints the full completion script
/// including helper functions.
///
/// # Errors
///
/// Returns an error if SSH config parsing fails (for `--list-hosts`).
#[allow(clippy::print_stdout)]
pub fn run(shell: &ShellType, list_hosts: bool) -> Result<i32> {
    if list_hosts {
        return print_hosts();
    }

    let mut cmd = Cli::command();

    match shell {
        ShellType::Bash => {
            clap_complete::generate(shells::Bash, &mut cmd, "fterm", &mut io::stdout());
            println!();
            print!("{}", bash_functions());
        }
        ShellType::Fish => {
            clap_complete::generate(shells::Fish, &mut cmd, "fterm", &mut io::stdout());
            println!();
            print!("{}", fish_functions());
        }
    }

    Ok(0)
}

/// Print all SSH host names, one per line.
fn print_hosts() -> Result<i32> {
    let ssh_home = get_dir();
    let config_path = ssh_home.join("config");

    if !config_path.exists() {
        return Ok(0);
    }

    let config_files = resolve_included_files(&config_path, &ssh_home)
        .context("failed to resolve SSH config includes")?;

    let host_list = hosts::get_all(&config_files).context("failed to get SSH host list")?;

    #[allow(clippy::print_stdout)]
    for host in &host_list {
        println!("{host}");
    }

    Ok(0)
}

/// Bash helper functions for shell integration.
const fn bash_functions() -> &'static str {
    r#"# fterm shell integration for bash

# Transparent wrappers — route through fterm for logging/tmux/MSYS2.
ssh() { command fterm ssh "$@"; }
scp() { command fterm scp "$@"; }
ssh-add() { command fterm ssh-add "$@"; }
ssh-keygen() { command fterm ssh-keygen "$@"; }

# Interactive SSH host selection — select a host, record in history, and execute.
fssh() {
    local result
    result=$(command fterm fssh 2>/dev/null)
    local status=$?
    if [ $status -eq 0 ] && [ -n "$result" ]; then
        history -s "$result"
        eval "$result"
    fi
    return $status
}

# Interactive log viewer — select a log file and open viewer.
flog() {
    local result
    result=$(command fterm flog 2>/dev/null)
    local status=$?
    if [ $status -eq 0 ] && [ -n "$result" ]; then
        history -s "$result"
        eval "$result"
    fi
    return $status
}

# SSH/SCP option and host completion for 'fterm ssh' and 'fterm scp'
_fterm_ssh_hosts() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local subcmd="${COMP_WORDS[1]}"
    if [[ "$subcmd" == "ssh" ]]; then
        if [[ "$cur" == -* ]]; then
            local ssh_opts="-4 -6 -A -a -B -b -C -c -D -E -e -F -f -G -g -I -i -J -K -k -L -l -M -m -N -n -O -o -P -p -Q -q -R -S -s -T -t -V -v -W -w -X -x -Y -y"
            COMPREPLY+=($(compgen -W "$ssh_opts" -- "$cur"))
        else
            local hosts
            hosts=$(command fterm completion bash --list-hosts 2>/dev/null)
            COMPREPLY+=($(compgen -W "$hosts" -- "$cur"))
        fi
    elif [[ "$subcmd" == "scp" ]]; then
        if [[ "$cur" == -* ]]; then
            local scp_opts="-3 -4 -6 -B -C -c -D -F -f -i -J -l -O -o -P -p -q -R -r -S -s -T -v"
            COMPREPLY+=($(compgen -W "$scp_opts" -- "$cur"))
        else
            local hosts
            hosts=$(command fterm completion bash --list-hosts 2>/dev/null)
            COMPREPLY+=($(compgen -S ':' -W "$hosts" -- "$cur"))
        fi
    fi
}
complete -F _fterm_ssh_hosts fterm
"#
}

/// Fish helper functions for shell integration.
const fn fish_functions() -> &'static str {
    r#"# fterm shell integration for fish

# Transparent wrappers — route through fterm for logging/tmux/MSYS2.
function ssh --wraps=ssh --description "SSH via fterm"
    command fterm ssh $argv
end
function scp --wraps=scp --description "SCP via fterm"
    command fterm scp $argv
end
function ssh-add --wraps=ssh-add --description "ssh-add via fterm"
    command fterm ssh-add $argv
end
function ssh-keygen --wraps=ssh-keygen --description "ssh-keygen via fterm"
    command fterm ssh-keygen $argv
end

# Interactive SSH host selection — places 'ssh <host>' on the command line.
function fssh --description "Interactive SSH host selection via fzf"
    set -l result (command fterm fssh 2>/dev/null)
    if test $status -eq 0 -a -n "$result"
        commandline $result
    end
end

# Interactive log viewer — places viewer command on the command line.
function flog --description "Interactive log viewer via fzf"
    set -l result (command fterm flog 2>/dev/null)
    if test $status -eq 0 -a -n "$result"
        commandline $result
    end
end

# Disable file completions for interactive commands
complete --command fssh --no-files
complete --command flog --no-files

# SSH host completion for 'fterm ssh'
complete -c fterm -n "__fish_seen_subcommand_from ssh" -f -a "(command fterm completion fish --list-hosts 2>/dev/null)"

# SCP host completion for 'fterm scp' (with colon suffix)
complete -c fterm -n "__fish_seen_subcommand_from scp" -f -a "(command fterm completion fish --list-hosts 2>/dev/null | sed 's/\$/:/')"
"#
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::undocumented_unsafe_blocks)]

    use std::env;

    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn bash_functions_contains_fssh_and_flog() {
        // Arrange / Act
        let output = bash_functions();

        // Assert
        assert!(output.contains("fssh()"));
        assert!(output.contains("flog()"));
        assert!(output.contains("history -s"));
        assert!(output.contains("eval"));
    }

    #[test]
    fn bash_functions_contains_transparent_wrappers() {
        // Arrange / Act
        let output = bash_functions();

        // Assert
        assert!(output.contains("ssh()"));
        assert!(output.contains("scp()"));
        assert!(output.contains("ssh-add()"));
        assert!(output.contains("ssh-keygen()"));
        assert!(output.contains("command fterm ssh"));
        assert!(output.contains("command fterm scp"));
        assert!(output.contains("command fterm ssh-add"));
        assert!(output.contains("command fterm ssh-keygen"));
    }

    #[test]
    fn fish_functions_contains_fssh_and_flog() {
        // Arrange / Act
        let output = fish_functions();

        // Assert
        assert!(output.contains("function fssh"));
        assert!(output.contains("function flog"));
        assert!(output.contains("commandline $result"));
        assert!(!output.contains("commandline -f execute"));
    }

    #[test]
    fn fish_functions_contains_transparent_wrappers() {
        // Arrange / Act
        let output = fish_functions();

        // Assert
        assert!(output.contains("function ssh"));
        assert!(output.contains("function scp"));
        assert!(output.contains("function ssh-add"));
        assert!(output.contains("function ssh-keygen"));
    }

    #[test]
    fn bash_fssh_flog_suppress_stderr() {
        // Arrange / Act
        let output = bash_functions();

        // Assert
        assert!(output.contains("fterm fssh 2>/dev/null"));
        assert!(output.contains("fterm flog 2>/dev/null"));
    }

    #[test]
    fn bash_ssh_option_completions() {
        // Arrange / Act
        let output = bash_functions();

        // Assert
        assert!(output.contains("ssh_opts="));
        assert!(output.contains("-p"));
        assert!(output.contains("-i"));
        assert!(output.contains("-v"));
    }

    #[test]
    fn bash_scp_option_completions() {
        // Arrange / Act
        let output = bash_functions();

        // Assert
        assert!(output.contains("scp_opts="));
        assert!(output.contains("-r"));
        assert!(output.contains("-P"));
    }

    #[test]
    fn bash_scp_host_completion_has_colon_suffix() {
        // Arrange / Act
        let output = bash_functions();

        // Assert
        assert!(output.contains("compgen -S ':'"));
    }

    #[test]
    fn fish_scp_host_completion_has_colon_suffix() {
        // Arrange / Act
        let output = fish_functions();

        // Assert
        assert!(output.contains("__fish_seen_subcommand_from scp"));
        assert!(output.contains("sed 's/\\$/:/'"));
    }

    // -----------------------------------------------------------------------
    // run() and print_hosts() integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn run_bash_returns_ok() {
        // Arrange / Act
        let result = run(&ShellType::Bash, false);

        // Assert
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn run_fish_returns_ok() {
        // Arrange / Act
        let result = run(&ShellType::Fish, false);

        // Assert
        assert_eq!(result.unwrap(), 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_list_hosts_no_config_returns_ok() {
        // Arrange — HOME points to a dir with no .ssh/config
        let tmp = TempDir::new().unwrap();
        let original = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("HOME", tmp.path().to_str().unwrap()) };
        unsafe { env::remove_var("MSYSTEM") };

        // Act
        let result = run(&ShellType::Bash, true);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            match &original {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — no config file means early Ok(0) return
        assert_eq!(result.unwrap(), 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_list_hosts_with_config_returns_ok() {
        // Arrange — create a temp .ssh/config with some Host entries
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        std::fs::write(
            ssh_dir.join("config"),
            "Host myserver\n  HostName 10.0.0.1\nHost staging\n  HostName 10.0.0.2\n",
        )
        .unwrap();

        let original = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("HOME", tmp.path().to_str().unwrap()) };
        unsafe { env::remove_var("MSYSTEM") };

        // Act
        let result = run(&ShellType::Fish, true);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            match &original {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — hosts printed to stdout, returns 0
        assert_eq!(result.unwrap(), 0);
    }
}
