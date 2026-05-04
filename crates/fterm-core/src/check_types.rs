//! Validation result types and formatting for SSH config checks.

/// Level of a validation message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    /// Hard failure that should block usage.
    Error,
    /// Soft issue that may indicate misconfiguration.
    Warn,
    /// Informational notice (not an error or warning).
    Info,
}

/// A single validation message.
#[derive(Debug, Clone)]
pub struct CheckMessage {
    /// Severity level.
    pub level: CheckLevel,
    /// Human-readable description.
    pub text: String,
}

/// Result of running all validation checks.
#[derive(Debug)]
pub struct ValidationResult {
    /// All collected messages.
    pub messages: Vec<CheckMessage>,
    /// Count of error-level messages.
    pub error_count: usize,
    /// Count of warning-level messages.
    pub warn_count: usize,
}

/// Format a coloured summary line for the validation result.
#[must_use]
pub fn format_summary(result: &ValidationResult) -> String {
    if result.error_count == 0 && result.warn_count == 0 {
        return String::from("\x1b[32m✓ All checks passed.\x1b[0m");
    }

    let mut parts: Vec<String> = Vec::new();
    if result.error_count > 0 {
        parts.push(format!("\x1b[31m{} error(s)\x1b[0m", result.error_count));
    }
    if result.warn_count > 0 {
        parts.push(format!("\x1b[33m{} warning(s)\x1b[0m", result.warn_count));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn format_summary_all_passed() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 0,
            warn_count: 0,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("All checks passed"));
    }

    #[test]
    fn format_summary_with_errors_and_warnings() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 2,
            warn_count: 3,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("2 error(s)"));
        assert!(summary.contains("3 warning(s)"));
    }

    #[test]
    fn format_summary_only_errors_no_warnings() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 3,
            warn_count: 0,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("3 error(s)"));
        assert!(!summary.contains("warning(s)"));
    }

    #[test]
    fn format_summary_only_warnings_no_errors() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 0,
            warn_count: 5,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("5 warning(s)"));
        assert!(!summary.contains("error(s)"));
    }
}
