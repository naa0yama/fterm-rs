//! fzf subprocess integration for interactive selection.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Options for configuring fzf behavior.
#[derive(Debug)]
pub struct Options {
    /// Height of the fzf window (e.g., "40%").
    pub height: String,
    /// Display items in reverse order.
    pub reverse: bool,
    /// Enable exact-match mode.
    pub exact: bool,
    /// Enable ANSI color code interpretation.
    pub ansi: bool,
    /// Prompt string shown to the user.
    pub prompt: String,
    /// Header text displayed below the input line.
    pub header: Option<String>,
    /// Shell command for the preview window.
    pub preview: Option<String>,
    /// Preview window layout (e.g., "right:50%").
    pub preview_window: Option<String>,
    /// Border style (e.g., "rounded").
    pub border: Option<String>,
    /// Margin (e.g., "0,1").
    pub margin: Option<String>,
    /// Field delimiter (e.g., ":").
    pub delimiter: Option<String>,
    /// Field index expression for search scope (e.g., "1").
    pub nth: Option<String>,
    /// Key bindings (each element is a single --bind argument).
    pub bind: Vec<String>,
}

/// Build fzf CLI arguments from options.
fn build_args(opts: &Options) -> Vec<String> {
    let mut args = Vec::new();

    args.push(String::from("--height"));
    args.push(opts.height.clone());

    if opts.reverse {
        args.push(String::from("--reverse"));
    }

    if opts.exact {
        args.push(String::from("--exact"));
    }

    if opts.ansi {
        args.push(String::from("--ansi"));
    }

    args.push(String::from("--prompt"));
    args.push(opts.prompt.clone());

    if let Some(ref header) = opts.header {
        args.push(String::from("--header"));
        args.push(header.clone());
    }

    if let Some(ref preview) = opts.preview {
        args.push(String::from("--preview"));
        args.push(preview.clone());
    }

    if let Some(ref pw) = opts.preview_window {
        args.push(String::from("--preview-window"));
        args.push(pw.clone());
    }

    if let Some(ref border) = opts.border {
        args.push(String::from("--border"));
        args.push(border.clone());
    }

    if let Some(ref margin) = opts.margin {
        args.push(String::from("--margin"));
        args.push(margin.clone());
    }

    if let Some(ref delimiter) = opts.delimiter {
        args.push(String::from("--delimiter"));
        args.push(delimiter.clone());
    }

    if let Some(ref nth) = opts.nth {
        args.push(String::from("--nth"));
        args.push(nth.clone());
    }

    for b in &opts.bind {
        args.push(String::from("--bind"));
        args.push(b.clone());
    }

    args
}

/// Run fzf with the given items and options.
///
/// Items are piped to fzf via stdin. The UI is drawn on stderr (inherited).
/// Returns `Ok(Some(selected))` on selection, `Ok(None)` on abort (ESC/Ctrl-C)
/// or no match.
///
/// # Errors
///
/// Returns an error if fzf is not installed or the subprocess fails unexpectedly.
pub fn run(items: &[String], opts: &Options) -> Result<Option<String>> {
    which::which("fzf").context("fzf is not installed")?;

    let args = build_args(opts);

    let mut child = Command::new("fzf")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn fzf")?;

    if let Some(ref mut stdin) = child.stdin {
        let input = items.join("\n");
        stdin
            .write_all(input.as_bytes())
            .context("failed to write items to fzf stdin")?;
    }
    // Close stdin so fzf can start processing.
    drop(child.stdin.take());

    let output = child.wait_with_output().context("failed to wait for fzf")?;

    let code = output.status.code().unwrap_or(1);
    match code {
        0 => {
            let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if selected.is_empty() {
                Ok(None)
            } else {
                Ok(Some(selected))
            }
        }
        // 1 = no match, 130 = abort (ESC/Ctrl-C)
        1 | 130 => Ok(None),
        other => anyhow::bail!("fzf exited with unexpected code: {other}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use super::*;

    fn default_opts() -> Options {
        Options {
            height: String::from("40%"),
            reverse: false,
            exact: false,
            ansi: false,
            prompt: String::from("Test> "),
            header: None,
            preview: None,
            preview_window: None,
            border: None,
            margin: None,
            delimiter: None,
            nth: None,
            bind: vec![],
        }
    }

    #[test]
    fn build_args_minimal() {
        // Arrange
        let opts = default_opts();

        // Act
        let args = build_args(&opts);

        // Assert
        assert_eq!(args, vec!["--height", "40%", "--prompt", "Test> "]);
    }

    #[test]
    fn build_args_with_reverse() {
        // Arrange
        let mut opts = default_opts();
        opts.reverse = true;

        // Act
        let args = build_args(&opts);

        // Assert
        assert!(args.contains(&String::from("--reverse")));
    }

    #[test]
    fn build_args_with_preview() {
        // Arrange
        let mut opts = default_opts();
        opts.preview = Some(String::from("cat {}"));
        opts.preview_window = Some(String::from("right:50%"));

        // Act
        let args = build_args(&opts);

        // Assert
        let preview_idx = args.iter().position(|a| a == "--preview").unwrap();
        assert_eq!(args[preview_idx + 1], "cat {}");
        let pw_idx = args.iter().position(|a| a == "--preview-window").unwrap();
        assert_eq!(args[pw_idx + 1], "right:50%");
    }

    #[test]
    fn build_args_with_border_and_margin() {
        // Arrange
        let mut opts = default_opts();
        opts.border = Some(String::from("rounded"));
        opts.margin = Some(String::from("0,1"));

        // Act
        let args = build_args(&opts);

        // Assert
        let border_idx = args.iter().position(|a| a == "--border").unwrap();
        assert_eq!(args[border_idx + 1], "rounded");
        let margin_idx = args.iter().position(|a| a == "--margin").unwrap();
        assert_eq!(args[margin_idx + 1], "0,1");
    }

    #[test]
    fn build_args_with_multiple_binds() {
        // Arrange
        let mut opts = default_opts();
        opts.bind = vec![
            String::from("ctrl-s:reload(rg {q})"),
            String::from("ctrl-f:reload(find .)"),
        ];

        // Act
        let args = build_args(&opts);

        // Assert
        let bind_indices: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--bind")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(bind_indices.len(), 2);
        assert_eq!(args[bind_indices[0] + 1], "ctrl-s:reload(rg {q})");
        assert_eq!(args[bind_indices[1] + 1], "ctrl-f:reload(find .)");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_with_empty_items_returns_none_or_err() {
        // Arrange — no items: fzf exits 1 (no match) in non-TTY, returns Ok(None)
        // In some environments fzf may exit 2 (usage error); treat both as acceptable.
        let opts = default_opts();

        // Act
        let result = run(&[], &opts);

        // Assert — either Ok(None) (no match) or Err (unexpected code) is acceptable;
        // Ok(Some(_)) would mean fzf selected something from an empty input, which is invalid.
        // NOTEST(infra): non-TTY fzf may emit exit 2 → Err arm is reachable in CI
        assert!(
            matches!(result, Ok(None) | Err(_)),
            "expected Ok(None) or Err for empty input"
        );
    }

    #[test]
    fn build_args_all_options() {
        // Arrange
        let opts = Options {
            height: String::from("80%"),
            reverse: true,
            exact: true,
            ansi: true,
            prompt: String::from("SSH> "),
            header: Some(String::from("Press ctrl-s to search")),
            preview: Some(String::from("echo {}")),
            preview_window: Some(String::from("right:60%")),
            border: Some(String::from("rounded")),
            margin: Some(String::from("1,2")),
            delimiter: Some(String::from(":")),
            nth: Some(String::from("1")),
            bind: vec![String::from("ctrl-a:toggle-all")],
        };

        // Act
        let args = build_args(&opts);

        // Assert
        assert_eq!(
            args,
            vec![
                "--height",
                "80%",
                "--reverse",
                "--exact",
                "--ansi",
                "--prompt",
                "SSH> ",
                "--header",
                "Press ctrl-s to search",
                "--preview",
                "echo {}",
                "--preview-window",
                "right:60%",
                "--border",
                "rounded",
                "--margin",
                "1,2",
                "--delimiter",
                ":",
                "--nth",
                "1",
                "--bind",
                "ctrl-a:toggle-all",
            ]
        );
    }
}
