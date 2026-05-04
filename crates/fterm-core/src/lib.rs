#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
//! Core traits, types, and pure functions for fterm.
//!
//! This crate contains no external command invocations (`std::process::Command`).
//! All code here is safe to test under Miri.

pub mod check_types;
pub mod runner;
pub mod ssh_parse;
pub mod util;
