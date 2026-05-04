//! CLI argument definitions for fterm.

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Version string including package name, version, and git hash.
pub const APP_VERSION: &str = concat!(
    env!("CARGO_PKG_NAME"),
    " version ",
    env!("CARGO_PKG_VERSION"),
    " (rev:",
    env!("GIT_HASH"),
    ")\n",
);

/// SSH/SCP connection management tool with fuzzy finder, tmux integration,
/// and config validation.
#[derive(Parser, Debug)]
#[command(about, version = APP_VERSION)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Interactive SSH host selection via fzf.
    Fssh,
    /// SSH wrapper with validation, logging, and tmux integration.
    Ssh(SshArgs),
    /// SCP wrapper with validation and logging.
    Scp(ScpArgs),
    /// Log viewer with fzf UI.
    Flog,
    /// SSH config template generator (interactive).
    Fgen,
    /// ANSI filter + timestamp for tmux pipe-pane (internal use).
    LogFilter,
    /// ssh-add wrapper (MSYS2: routes to Windows OpenSSH).
    SshAdd(SshAddArgs),
    /// ssh-keygen wrapper (MSYS2: routes to Windows OpenSSH).
    SshKeygen(SshKeygenArgs),
    /// Generate shell completions and helper functions.
    Completion(CompletionArgs),
}

/// Arguments for shell completion generation.
#[derive(Args, Debug)]
pub struct CompletionArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: ShellType,

    /// Print SSH host list for completion (one per line).
    #[arg(long)]
    pub list_hosts: bool,
}

/// Supported shells for completion generation.
#[derive(ValueEnum, Clone, Debug)]
pub enum ShellType {
    /// Bash shell.
    Bash,
    /// Fish shell.
    Fish,
}

/// Arguments passed through to the ssh command.
#[derive(Args, Debug)]
pub struct SshArgs {
    /// Arguments forwarded to ssh.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// Arguments passed through to the scp command.
#[derive(Args, Debug)]
pub struct ScpArgs {
    /// Arguments forwarded to scp.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// Arguments passed through to the ssh-add command.
#[derive(Args, Debug)]
pub struct SshAddArgs {
    /// Arguments forwarded to ssh-add.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// Arguments passed through to the ssh-keygen command.
#[derive(Args, Debug)]
pub struct SshKeygenArgs {
    /// Arguments forwarded to ssh-keygen.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}
