//! SSH config template generator (interactive).
//!
//! Reads `~/.ssh/template.conf`, prompts for organization and environment
//! names, applies substitutions, and writes the generated config file.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::debug;

use crate::config::home::get_dir;

/// Default template content created when no template exists.
const DEFAULT_TEMPLATE: &str = r"# SSH config template for fterm
# Placeholders: org.dev, org.env, org_dev, org_env
#
# Usage: fterm fgen
#
Host org.dev-*
  User deploy
  IdentityFile ~/.ssh/keys/org_dev/id_ed25519
  ProxyJump org.dev-bastion
  StrictHostKeyChecking yes
  IdentitiesOnly yes
";

/// Run the SSH config template generator.
///
/// Checks for `~/.ssh/template.conf`, creates a default if missing, then
/// prompts the user for organization and environment names. Applies
/// sed-like substitutions and writes the result to the appropriate config
/// directory.
///
/// # Errors
///
/// Returns an error if file I/O or user input fails.
pub fn run() -> Result<i32> {
    let ssh_home = get_dir();
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut writer = io::stdout();
    run_inner(&ssh_home, &mut reader, &mut writer)
}

/// Core logic for the SSH config template generator, with injected I/O.
///
/// Accepts `ssh_home` path, a reader for user input, and a writer for output,
/// enabling deterministic testing without real stdin/filesystem side effects.
///
/// # Errors
///
/// Returns an error if file I/O or user input fails.
#[tracing::instrument(skip(reader, writer), err)]
fn run_inner(ssh_home: &Path, reader: &mut dyn BufRead, writer: &mut dyn Write) -> Result<i32> {
    let template_path = ssh_home.join("template.conf");

    if ensure_template(&template_path, writer)? {
        return Ok(0);
    }

    // Prompt for organization name (retry up to 3 times)
    let org = prompt_required("Organization (e.g., mycompany)", reader, writer)?;
    if org.is_empty() {
        writeln!(writer, "Error: Organization name cannot be empty.")
            .context("failed to write output")?;
        return Ok(1);
    }

    // Prompt for environment name (retry up to 3 times)
    let env_name = prompt_required("Environment (e.g., dev, stg, prod)", reader, writer)?;
    if env_name.is_empty() {
        writeln!(writer, "Error: Environment name cannot be empty.")
            .context("failed to write output")?;
        return Ok(1);
    }

    debug!(org = %org, env = %env_name, "generating SSH config from template");

    // Read template
    let template = fs::read_to_string(&template_path)
        .with_context(|| format!("failed to read template: {}", template_path.display()))?;

    // Apply substitutions
    let output = apply_substitutions(&template, &org, &env_name);

    // Write output file
    let output_dir = ssh_home.join("conf.d").join("envs").join(&org);
    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    let output_path = output_dir.join(format!("{env_name}.conf"));

    if !write_config(&output_path, &output, reader, writer)? {
        return Ok(1);
    }

    writeln!(writer, "Generated: {}", output_path.display()).context("failed to write output")?;
    writeln!(writer).context("failed to write output")?;
    writeln!(writer, "Preview (first 20 lines):").context("failed to write output")?;

    // Show preview
    show_preview(&output, writer)?;

    Ok(0)
}

/// Apply the four sed-like substitutions to the template.
fn apply_substitutions(template: &str, org: &str, env: &str) -> String {
    let org_dot_env = format!("{org}.{env}");
    let org_underscore_env = format!("{org}_{env}");

    template
        .replace("org.dev", &org_dot_env)
        .replace("org.env", &org_dot_env)
        .replace("org_dev", &org_underscore_env)
        .replace("org_env", &org_underscore_env)
}

/// Prompt the user for a required (non-empty) value, retrying up to 3 times.
///
/// Returns an empty string if the user provides no input after retries.
///
/// # Errors
///
/// Returns an error if reading from stdin or writing the prompt fails.
fn prompt_required(
    label: &str,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<String> {
    for attempt in 0..3_u8 {
        let value = prompt_input(label, reader, writer)?;
        if !value.is_empty() {
            return Ok(value);
        }
        if attempt < 2 {
            writeln!(writer, "Input cannot be empty. Please try again.")
                .context("failed to write output")?;
        }
    }
    Ok(String::new())
}

/// Prompt the user for input with the given label.
///
/// # Errors
///
/// Returns an error if reading from stdin or writing the prompt fails.
fn prompt_input(label: &str, reader: &mut dyn BufRead, writer: &mut dyn Write) -> Result<String> {
    write!(writer, "{label}: ").context("failed to flush stdout")?;
    writer.flush().context("failed to flush stdout")?;

    let mut input = String::new();
    reader
        .read_line(&mut input)
        .context("failed to read user input")?;

    Ok(input.trim().to_owned())
}

/// Atomically create the default template if it does not already exist.
///
/// Returns `true` if a new template was created (caller should exit early),
/// or `false` if the template already existed.
///
/// # Errors
///
/// Returns an error if file creation or writing fails.
fn ensure_template(template_path: &Path, writer: &mut dyn Write) -> Result<bool> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(template_path)
    {
        Ok(mut file) => {
            writeln!(
                writer,
                "Creating default template: {}",
                template_path.display()
            )
            .context("failed to write output")?;
            file.write_all(DEFAULT_TEMPLATE.as_bytes())
                .with_context(|| {
                    format!(
                        "failed to create default template: {}",
                        template_path.display()
                    )
                })?;
            writeln!(
                writer,
                "Template created. Please edit it before running fgen again."
            )
            .context("failed to write output")?;
            Ok(true)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e).with_context(|| {
            format!(
                "failed to create default template: {}",
                template_path.display()
            )
        }),
    }
}

/// Write the generated config, prompting for confirmation if the file exists.
///
/// Returns `true` if the file was written, or `false` if the user declined to
/// overwrite.
///
/// # Errors
///
/// Returns an error if file I/O or user input fails.
fn write_config(
    output_path: &Path,
    output: &str,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<bool> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
    {
        Ok(mut file) => {
            file.write_all(output.as_bytes()).with_context(|| {
                format!(
                    "failed to write generated config: {}",
                    output_path.display()
                )
            })?;
            Ok(true)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            write!(
                writer,
                "File already exists: {}. Overwrite? [y/N]: ",
                output_path.display()
            )
            .context("failed to write output")?;
            writer.flush().context("failed to flush output")?;
            let mut answer = String::new();
            reader
                .read_line(&mut answer)
                .context("failed to read user input")?;
            if !answer.trim().eq_ignore_ascii_case("y") {
                writeln!(writer, "Aborted.").context("failed to write output")?;
                return Ok(false);
            }
            fs::write(output_path, output).with_context(|| {
                format!(
                    "failed to write generated config: {}",
                    output_path.display()
                )
            })?;
            Ok(true)
        }
        Err(e) => Err(e).with_context(|| {
            format!(
                "failed to write generated config: {}",
                output_path.display()
            )
        }),
    }
}

/// Show the first 20 lines of the generated config.
///
/// # Errors
///
/// Returns an error if writing output fails.
fn show_preview(content: &str, writer: &mut dyn Write) -> Result<()> {
    for (i, line) in content.lines().enumerate() {
        if i >= 20 {
            writeln!(writer, "  ...").context("failed to write preview")?;
            break;
        }
        writeln!(writer, "  {line}").context("failed to write preview")?;
    }
    Ok(())
}

/// Get the output path for a generated config file.
///
/// Returns `{ssh_home}/conf.d/envs/{org}/{env}.conf`.
#[must_use]
pub fn get_output_path(ssh_home: &Path, org: &str, env: &str) -> PathBuf {
    ssh_home
        .join("conf.d")
        .join("envs")
        .join(org)
        .join(format!("{env}.conf"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::io::Cursor;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn apply_substitutions_replaces_all_patterns() {
        // Arrange
        let template = "Host org.dev-web\n  IdentityFile ~/.ssh/org_dev/k\n  ProxyJump org.env-bastion\n  Path org_env\n";

        // Act
        let result = apply_substitutions(template, "myco", "prod");

        // Assert
        assert!(result.contains("Host myco.prod-web"));
        assert!(!result.contains("org_dev") || result.contains("myco_prod"));
        assert!(result.contains("myco.prod-bastion"));
        assert!(result.contains("myco_prod"));
    }

    #[test]
    fn apply_substitutions_preserves_non_matching_text() {
        // Arrange
        let template = "Host static-host\n  User admin\n";

        // Act
        let result = apply_substitutions(template, "myco", "staging");

        // Assert
        assert_eq!(result, "Host static-host\n  User admin\n");
    }

    #[test]
    fn get_output_path_builds_correct_path() {
        // Arrange
        let ssh_home = Path::new("/home/user/.ssh");

        // Act
        let path = get_output_path(ssh_home, "myco", "prod");

        // Assert
        assert_eq!(
            path,
            PathBuf::from("/home/user/.ssh/conf.d/envs/myco/prod.conf")
        );
    }

    #[test]
    fn show_preview_short_content() {
        // Arrange
        let content = "line1\nline2\nline3\n";
        let mut output = Vec::new();

        // Act
        show_preview(content, &mut output).unwrap();

        // Assert
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("  line1"));
        assert!(text.contains("  line3"));
        assert!(!text.contains("..."));
    }

    #[test]
    fn show_preview_long_content() {
        // Arrange
        use std::fmt::Write as FmtWrite;
        let mut content = String::new();
        for i in 0..30 {
            writeln!(content, "line {i}").unwrap();
        }
        let mut output = Vec::new();

        // Act
        show_preview(&content, &mut output).unwrap();

        // Assert
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("  line 0"));
        assert!(text.contains("  line 19"));
        assert!(text.contains("  ..."));
        assert!(!text.contains("  line 20"));
    }

    #[test]
    fn show_preview_empty_content() {
        // Arrange
        let content = "";
        let mut output = Vec::new();

        // Act
        show_preview(content, &mut output).unwrap();

        // Assert
        let text = String::from_utf8(output).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn apply_substitutions_empty_template() {
        // Arrange
        let template = "";

        // Act
        let result = apply_substitutions(template, "myco", "prod");

        // Assert
        assert_eq!(result, "");
    }

    #[test]
    fn apply_substitutions_multiple_occurrences() {
        // Arrange
        let template = "Host org.dev-web\nProxy org.dev-bastion\nAlias org.dev\n";

        // Act
        let result = apply_substitutions(template, "acme", "stg");

        // Assert
        assert!(!result.contains("org.dev"));
        assert_eq!(result.matches("acme.stg").count(), 3);
    }

    #[test]
    fn default_template_is_valid() {
        // Arrange / Act
        let result = apply_substitutions(DEFAULT_TEMPLATE, "testorg", "staging");

        // Assert
        assert!(result.contains("testorg.staging"));
        assert!(result.contains("testorg_staging"));
        assert!(!result.contains("org.dev"));
        assert!(!result.contains("org_dev"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_creates_default_template_and_exits_early() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        let mut reader = Cursor::new(b"");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert — should exit early after creating template
        assert_eq!(code, 0);
        let template_path = ssh_home.join("template.conf");
        assert!(template_path.exists());
        let content = fs::read_to_string(&template_path).unwrap();
        assert_eq!(content, DEFAULT_TEMPLATE);
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Template created"));
        assert!(output.contains("edit it before running fgen again"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_empty_org_returns_1() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        // Pre-create template so it does not exit early
        fs::write(ssh_home.join("template.conf"), DEFAULT_TEMPLATE).unwrap();
        // 3 empty lines to exhaust retries
        let mut reader = Cursor::new(b"\n\n\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 1);
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Organization name cannot be empty"));
        assert!(output.contains("Input cannot be empty"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_empty_env_returns_1() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        // Pre-create template so it does not exit early
        fs::write(ssh_home.join("template.conf"), DEFAULT_TEMPLATE).unwrap();
        // org succeeds, then 3 empty lines for env
        let mut reader = Cursor::new(b"org\n\n\n\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 1);
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Environment name cannot be empty"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_retry_on_empty_org_then_success() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        fs::write(ssh_home.join("template.conf"), DEFAULT_TEMPLATE).unwrap();
        // First empty, then valid org and env
        let mut reader = Cursor::new(b"\nmyco\nprod\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 0);
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Input cannot be empty"));
        assert!(output.contains("Generated:"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_generates_config_file() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        let template = "Host org.dev-*\n  User org_dev\n";
        fs::write(ssh_home.join("template.conf"), template).unwrap();
        let mut reader = Cursor::new(b"myco\nprod\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 0);
        let output_path = ssh_home.join("conf.d/envs/myco/prod.conf");
        assert!(output_path.exists());
        let generated = fs::read_to_string(&output_path).unwrap();
        assert!(generated.contains("Host myco.prod-*"));
        assert!(generated.contains("User myco_prod"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_uses_existing_template() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        let custom_template = "# Custom\nHost org.env-app\n  IdentityFile org_env.pem\n";
        fs::write(ssh_home.join("template.conf"), custom_template).unwrap();
        let mut reader = Cursor::new(b"acme\nstaging\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 0);
        let output_path = ssh_home.join("conf.d/envs/acme/staging.conf");
        let generated = fs::read_to_string(&output_path).unwrap();
        assert!(generated.contains("# Custom"));
        assert!(generated.contains("Host acme.staging-app"));
        assert!(generated.contains("IdentityFile acme_staging.pem"));
        // Should not have created default template message in output
        let output = String::from_utf8(writer).unwrap();
        assert!(!output.contains("Creating default template"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_overwrite_confirmed() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        let template = "Host org.dev-*\n  User org_dev\n";
        fs::write(ssh_home.join("template.conf"), template).unwrap();
        // First run
        let output_dir = ssh_home.join("conf.d/envs/myco");
        fs::create_dir_all(&output_dir).unwrap();
        fs::write(output_dir.join("prod.conf"), "old content").unwrap();
        // Input: org, env, then "y" for overwrite
        let mut reader = Cursor::new(b"myco\nprod\ny\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 0);
        let generated = fs::read_to_string(output_dir.join("prod.conf")).unwrap();
        assert!(generated.contains("Host myco.prod-*"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_overwrite_declined() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        let template = "Host org.dev-*\n  User org_dev\n";
        fs::write(ssh_home.join("template.conf"), template).unwrap();
        let output_dir = ssh_home.join("conf.d/envs/myco");
        fs::create_dir_all(&output_dir).unwrap();
        fs::write(output_dir.join("prod.conf"), "old content").unwrap();
        // Input: org, env, then "n" for overwrite
        let mut reader = Cursor::new(b"myco\nprod\nn\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 1);
        let content = fs::read_to_string(output_dir.join("prod.conf")).unwrap();
        assert_eq!(content, "old content");
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("Aborted"));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_inner_show_preview_truncates_long_output() {
        use std::fmt::Write as FmtWrite;

        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        let mut template = String::new();
        for i in 0..30 {
            writeln!(template, "Line{i} org.dev").unwrap();
        }
        fs::write(ssh_home.join("template.conf"), &template).unwrap();
        let mut reader = Cursor::new(b"co\ndev\n");
        let mut writer = Vec::new();

        // Act
        let code = run_inner(ssh_home, &mut reader, &mut writer).unwrap();

        // Assert
        assert_eq!(code, 0);
        let output = String::from_utf8(writer).unwrap();
        assert!(output.contains("..."));
        assert!(output.contains("Preview (first 20 lines):"));
    }
}
