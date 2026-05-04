//! Pure parsers for `ssh -G` output.

/// Parse a single value from `ssh -G` output by key (case-insensitive).
#[must_use]
pub fn parse_ssh_g_value(output: &str, key: &str) -> Option<String> {
    let key_lower = key.to_lowercase();
    for line in output.lines() {
        if line
            .split_once(' ')
            .is_some_and(|(k, _)| k.to_lowercase() == key_lower)
        {
            let (_, v) = line.split_once(' ')?;
            return Some(String::from(v));
        }
    }
    None
}

/// Parse all values for a given key from `ssh -G` output (case-insensitive).
#[must_use]
pub fn parse_ssh_g_values(output: &str, key: &str) -> Vec<String> {
    let key_lower = key.to_lowercase();
    output
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(' ')?;
            (k.to_lowercase() == key_lower).then(|| String::from(v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn parse_ssh_g_value_finds_key() {
        // Arrange
        let output = "hostname example.com\nport 22\nuser admin\n";

        // Act
        let hostname = parse_ssh_g_value(output, "hostname");
        let port = parse_ssh_g_value(output, "port");

        // Assert
        assert_eq!(hostname, Some(String::from("example.com")));
        assert_eq!(port, Some(String::from("22")));
    }

    #[test]
    fn parse_ssh_g_value_case_insensitive() {
        // Arrange
        let output = "HostName example.com\n";

        // Act
        let result = parse_ssh_g_value(output, "hostname");

        // Assert
        assert_eq!(result, Some(String::from("example.com")));
    }

    #[test]
    fn parse_ssh_g_value_returns_none_for_missing_key() {
        // Arrange
        let output = "hostname example.com\n";

        // Act
        let result = parse_ssh_g_value(output, "user");

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn parse_ssh_g_values_collects_all() {
        // Arrange
        let output = "identityfile ~/.ssh/id_rsa\nidentityfile ~/.ssh/id_ed25519\n";

        // Act
        let result = parse_ssh_g_values(output, "identityfile");

        // Assert
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "~/.ssh/id_rsa");
        assert_eq!(result[1], "~/.ssh/id_ed25519");
    }

    #[test]
    fn parse_ssh_g_value_empty_output_returns_none() {
        // Arrange
        let output = "";

        // Act
        let result = parse_ssh_g_value(output, "hostname");

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn parse_ssh_g_values_empty_output_returns_empty_vec() {
        // Arrange
        let output = "";

        // Act
        let result = parse_ssh_g_values(output, "identityfile");

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn parse_ssh_g_values_no_matching_key_returns_empty_vec() {
        // Arrange
        let output = "hostname example.com\nport 22\nuser admin\n";

        // Act
        let result = parse_ssh_g_values(output, "identityfile");

        // Assert
        assert!(result.is_empty());
    }
}
