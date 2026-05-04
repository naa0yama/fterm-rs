//! SSH argument parsing for accurate hostname extraction.
//!
//! Fixes the Fish implementation bug (Q2) where `$argv[-1]` was used,
//! which fails when a remote command follows the hostname.

/// SSH options that consume the next argument as a value.
const SSH_OPTIONS_WITH_VALUE: &[&str] = &[
    "-B", "-b", "-c", "-D", "-E", "-e", "-F", "-I", "-i", "-J", "-L", "-l", "-m", "-O", "-o", "-p",
    "-Q", "-R", "-S", "-W", "-w",
];

/// Extract the target hostname from SSH command arguments.
///
/// Parses the argument list following OpenSSH conventions:
/// - Skips options and their values
/// - First non-option argument is the destination (`[user@]hostname`)
/// - Remaining arguments are the remote command (ignored)
///
/// Returns `None` if no hostname is found.
#[must_use]
pub fn extract_ssh_host(args: &[String]) -> Option<String> {
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        // Option that takes a value
        if SSH_OPTIONS_WITH_VALUE.contains(&arg.as_str()) {
            skip_next = true;
            continue;
        }

        // Combined option+value (e.g., -p22, -oStrictHostKeyChecking=no)
        if arg.starts_with('-')
            && arg.len() > 2
            && SSH_OPTIONS_WITH_VALUE
                .iter()
                .any(|opt| arg.starts_with(opt))
        {
            continue;
        }

        // Regular flag (e.g., -v, -N, -T, -4, -6)
        if arg.starts_with('-') {
            continue;
        }

        // First non-option argument is the destination
        return Some(arg.clone());
    }

    None
}

/// Extract only the hostname portion from a destination string.
///
/// Handles `user@hostname` format by stripping the user prefix.
#[must_use]
pub fn extract_hostname_from_destination(destination: &str) -> &str {
    destination
        .rsplit_once('@')
        .map_or(destination, |(_, host)| host)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{extract_hostname_from_destination, extract_ssh_host};

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| String::from(*a)).collect()
    }

    #[test]
    fn simple_hostname() {
        let result = extract_ssh_host(&s(&["myhost"]));
        assert_eq!(result, Some(String::from("myhost")));
    }

    #[test]
    fn hostname_with_user() {
        let result = extract_ssh_host(&s(&["user@myhost"]));
        assert_eq!(result, Some(String::from("user@myhost")));
    }

    #[test]
    fn hostname_after_port_option() {
        let result = extract_ssh_host(&s(&["-p", "22", "myhost"]));
        assert_eq!(result, Some(String::from("myhost")));
    }

    #[test]
    fn hostname_after_combined_option() {
        let result = extract_ssh_host(&s(&["-p22", "myhost"]));
        assert_eq!(result, Some(String::from("myhost")));
    }

    #[test]
    fn hostname_with_remote_command() {
        // Bug fix Q2: ssh -p 22 hostname command
        let result = extract_ssh_host(&s(&["-p", "22", "hostname", "ls", "-la"]));
        assert_eq!(result, Some(String::from("hostname")));
    }

    #[test]
    fn hostname_after_flag_options() {
        let result = extract_ssh_host(&s(&["-v", "-N", "-T", "myhost"]));
        assert_eq!(result, Some(String::from("myhost")));
    }

    #[test]
    fn hostname_with_identity_and_jump() {
        let result = extract_ssh_host(&s(&["-i", "/key", "-J", "bastion", "target"]));
        assert_eq!(result, Some(String::from("target")));
    }

    #[test]
    fn hostname_with_o_option() {
        let result = extract_ssh_host(&s(&["-oStrictHostKeyChecking=no", "myhost"]));
        assert_eq!(result, Some(String::from("myhost")));
    }

    #[test]
    fn no_hostname() {
        let result = extract_ssh_host(&s(&["-V"]));
        assert_eq!(result, None);
    }

    #[test]
    fn empty_args() {
        let result = extract_ssh_host(&[]);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_host_from_user_at_host() {
        assert_eq!(extract_hostname_from_destination("user@myhost"), "myhost");
    }

    #[test]
    fn extract_host_plain() {
        assert_eq!(extract_hostname_from_destination("myhost"), "myhost");
    }
}
