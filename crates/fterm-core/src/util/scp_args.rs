//! SCP argument parsing for host extraction.

/// Extract unique remote hosts from SCP arguments.
///
/// Skips option flags (starting with `-`) and extracts the host portion
/// from arguments containing `:` (the `host:path` format).
#[must_use]
pub fn extract_hosts(args: &[String]) -> Vec<String> {
    extract_user_host_pairs(args)
        .into_iter()
        .map(|(_, host)| host)
        .collect()
}

/// Extract remote (optional-user, host) pairs from SCP arguments.
///
/// Returns each unique host with its explicitly specified user (if any).
/// For `user@host:path`, returns `(Some("user"), "host")`.
/// For `host:path`, returns `(None, "host")`.
/// Preserves first-occurrence order; duplicates (by host) are skipped.
#[must_use]
pub fn extract_user_host_pairs(args: &[String]) -> Vec<(Option<String>, String)> {
    let mut pairs: Vec<(Option<String>, String)> = Vec::new();
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        // Options that take a value argument
        if matches!(
            arg.as_str(),
            "-c" | "-D" | "-F" | "-i" | "-J" | "-l" | "-o" | "-P" | "-S" | "-X" | "-Y" | "-Z"
        ) {
            skip_next = true;
            continue;
        }

        // Skip flag-only options
        if arg.starts_with('-') {
            continue;
        }

        // Extract host from user@host:path or host:path
        if let Some(colon_pos) = arg.find(':') {
            let host_part = &arg[..colon_pos];
            if !host_part.is_empty() {
                let (user, host) = host_part.rsplit_once('@').map_or_else(
                    || (None, host_part.to_owned()),
                    |(u, h)| (Some(u.to_owned()), h.to_owned()),
                );
                let already_seen = pairs.iter().any(|(_, h)| h == &host);
                if !host.is_empty() && !already_seen {
                    pairs.push((user, host));
                }
            }
        }
    }

    pairs
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{extract_hosts, extract_user_host_pairs};

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| String::from(*a)).collect()
    }

    #[test]
    fn single_remote_host() {
        let result = extract_hosts(&s(&["local.txt", "server1:/remote/path"]));
        assert_eq!(result, vec!["server1"]);
    }

    #[test]
    fn user_at_host() {
        let result = extract_hosts(&s(&["user@server2:/path", "local.txt"]));
        assert_eq!(result, vec!["server2"]);
    }

    #[test]
    fn multiple_hosts_deduped() {
        let result = extract_hosts(&s(&["server1:/a", "server2:/b", "server1:/c"]));
        assert_eq!(result, vec!["server1", "server2"]);
    }

    #[test]
    fn local_file_only() {
        let result = extract_hosts(&s(&["local1.txt", "local2.txt"]));
        assert!(result.is_empty());
    }

    #[test]
    fn skip_option_flags() {
        let result = extract_hosts(&s(&["-P", "22", "-r", "server:/path"]));
        assert_eq!(result, vec!["server"]);
    }

    #[test]
    fn empty_args() {
        let result = extract_hosts(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn skip_identity_option() {
        let result = extract_hosts(&s(&["-i", "/key", "host:/path"]));
        assert_eq!(result, vec!["host"]);
    }

    // extract_user_host_pairs tests

    #[test]
    fn pairs_no_user() {
        let result = extract_user_host_pairs(&s(&["host1:/path", "local.txt"]));
        assert_eq!(result, vec![(None, String::from("host1"))]);
    }

    #[test]
    fn pairs_with_explicit_user() {
        let result = extract_user_host_pairs(&s(&["alice@host1:/path"]));
        assert_eq!(
            result,
            vec![(Some(String::from("alice")), String::from("host1"))]
        );
    }

    #[test]
    fn pairs_mixed_user_and_no_user() {
        let result = extract_user_host_pairs(&s(&["alice@host1:/a", "host2:/b"]));
        assert_eq!(
            result,
            vec![
                (Some(String::from("alice")), String::from("host1")),
                (None, String::from("host2")),
            ]
        );
    }

    #[test]
    fn pairs_dedup_by_host() {
        let result = extract_user_host_pairs(&s(&["host1:/a", "alice@host1:/b"]));
        // First occurrence wins
        assert_eq!(result, vec![(None, String::from("host1"))]);
    }

    #[test]
    fn pairs_empty_args() {
        let result = extract_user_host_pairs(&[]);
        assert!(result.is_empty());
    }
}
