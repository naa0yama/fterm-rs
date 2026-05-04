//! Utility functions.
//!
//! Pure utilities (no external command calls) live in `fterm-core::util`.
//! This module re-exports them and adds impure utilities that depend on
//! external commands (`files`, `fzf`).

pub use fterm_core::util::dry_run;
pub use fterm_core::util::duration;
pub use fterm_core::util::log_dir;
pub use fterm_core::util::path;
pub use fterm_core::util::scp_args;
pub use fterm_core::util::splash;
pub use fterm_core::util::ssh_args;
pub use fterm_core::util::ssh_env;

pub mod files;
pub mod fzf;
