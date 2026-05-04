//! SSH session management for fterm: tmux, logging, and file utilities.
//!
//! Uses `fterm-core::runner::CommandRunner` trait for all external command
//! calls — no direct `std::process::Command` usage.

pub mod logging;
pub mod tmux;
pub mod util;
