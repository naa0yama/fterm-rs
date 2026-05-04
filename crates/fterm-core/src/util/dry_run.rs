//! Dry-run detection for SSH and SCP commands.

/// Check if the SSH arguments indicate a dry-run invocation.
///
/// Dry-run flags: `-G`, `-V`, `-Q`, `--help`.
#[must_use]
pub fn is_ssh(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "-G" | "-V" | "-Q" | "--help"))
}

/// Check if the SCP arguments indicate a dry-run invocation.
///
/// Dry-run flags: `--help`, `-h`.
#[must_use]
pub fn is_scp(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{is_scp, is_ssh};

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| String::from(*a)).collect()
    }

    #[test]
    fn ssh_no_dry_run() {
        assert!(!is_ssh(&s(&["hostname"])));
        assert!(!is_ssh(&s(&["-p", "22", "hostname"])));
    }

    #[test]
    fn ssh_dry_run_g_flag() {
        assert!(is_ssh(&s(&["-G", "hostname"])));
    }

    #[test]
    fn ssh_dry_run_v_flag() {
        assert!(is_ssh(&s(&["-V"])));
    }

    #[test]
    fn ssh_dry_run_q_flag() {
        assert!(is_ssh(&s(&["-Q", "cipher"])));
    }

    #[test]
    fn ssh_dry_run_help() {
        assert!(is_ssh(&s(&["--help"])));
    }

    #[test]
    fn ssh_empty_args() {
        assert!(!is_ssh(&[]));
    }

    #[test]
    fn scp_no_dry_run() {
        assert!(!is_scp(&s(&["file", "host:path"])));
    }

    #[test]
    fn scp_dry_run_help() {
        assert!(is_scp(&s(&["--help"])));
    }

    #[test]
    fn scp_dry_run_h_flag() {
        assert!(is_scp(&s(&["-h"])));
    }

    #[test]
    fn scp_empty_args() {
        assert!(!is_scp(&[]));
    }
}
