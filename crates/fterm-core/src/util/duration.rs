//! Duration formatting for connection time display.

use std::fmt::Write;

/// Format a duration in seconds into a human-readable string.
///
/// Output format: `{d}d {h}h{m}m{s}s`, showing only non-zero leading units.
/// Examples: `0s`, `45s`, `5m30s`, `1h1m5s`, `3d 23h37m29s`.
#[must_use]
pub fn format(total_secs: u64) -> String {
    if total_secs == 0 {
        return String::from("0s");
    }

    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    let mut result = String::new();

    if days > 0 {
        let _ = write!(result, "{days}d ");
    }
    if days > 0 || hours > 0 {
        let _ = write!(result, "{hours}h");
    }
    if days > 0 || hours > 0 || minutes > 0 {
        let _ = write!(result, "{minutes}m");
    }
    let _ = write!(result, "{seconds}s");

    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::format;

    #[test]
    fn zero_seconds() {
        assert_eq!(format(0), "0s");
    }

    #[test]
    fn only_seconds() {
        assert_eq!(format(45), "45s");
    }

    #[test]
    fn minutes_and_seconds() {
        assert_eq!(format(330), "5m30s");
    }

    #[test]
    fn hours_minutes_seconds() {
        assert_eq!(format(3665), "1h1m5s");
    }

    #[test]
    fn days_hours_minutes_seconds() {
        // 3d=259200, 23h=82800, 37m=2220, 29s => 344249
        assert_eq!(format(344_249), "3d 23h37m29s");
    }

    #[test]
    fn exact_one_day() {
        assert_eq!(format(86400), "1d 0h0m0s");
    }

    #[test]
    fn exact_one_hour() {
        assert_eq!(format(3600), "1h0m0s");
    }

    #[test]
    fn exact_one_minute() {
        assert_eq!(format(60), "1m0s");
    }

    #[test]
    fn one_second() {
        assert_eq!(format(1), "1s");
    }
}
