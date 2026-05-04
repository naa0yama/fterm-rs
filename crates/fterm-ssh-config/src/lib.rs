//! SSH config parsing and validation for fterm.
//!
//! Uses `fterm-core::runner::CommandRunner` trait for all external command
//! calls — no direct `std::process::Command` usage.

pub mod config;
pub mod validate;
