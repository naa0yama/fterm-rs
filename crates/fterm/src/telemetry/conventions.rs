//! fterm-prefixed semantic conventions for app-specific telemetry.
//!
//! Mirrors the layout of `opentelemetry_semantic_conventions::{metric,
//! attribute}` to provide a single source of truth for `fterm.*` names
//! across all signals. Use these constants instead of string literals to
//! avoid typos and drift.

/// Metric name constants for fterm-specific instruments.
#[cfg(feature = "otel")]
pub mod metric {
    /// End-to-end subcommand execution latency in seconds.
    pub const COMMAND_DURATION: &str = "fterm.command.duration";
    /// Count of subcommand invocations that resulted in an error.
    pub const COMMAND_ERRORS: &str = "fterm.command.errors";
}

/// Attribute key constants for fterm-specific telemetry.
#[cfg(feature = "otel")]
pub mod attribute {
    /// The fterm subcommand name (e.g., `"ssh"`, `"flog"`).
    pub const COMMAND: &str = "fterm.command";
    /// Short error-type label derived from the error chain (e.g., `"io"`, `"unknown"`).
    pub const ERROR_KIND: &str = "fterm.error.kind";
}
