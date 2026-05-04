//! Log viewer with fzf UI.
//!
//! Presents log files in an interactive fzf interface with two modes:
//! file browsing and content search (via rg).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::debug;

use crate::util::files;
use crate::util::fzf;
use crate::util::log_dir;

/// Run the log viewer command.
///
/// Checks that the log directory exists and launches fzf for interactive
/// log file selection. Prints a viewer command to stdout.
///
/// # Errors
///
/// Returns an error if the log directory is missing or fzf invocation fails.
pub fn run() -> Result<i32> {
    run_inner(run_fzf_log_selection)
}

/// Core logic for the log viewer, parameterised by a selection function.
///
/// `select_fn` receives the log file paths and the log directory prefix,
/// returning the selected path (if any).
#[tracing::instrument(skip(select_fn), err)]
fn run_inner<F>(select_fn: F) -> Result<i32>
where
    F: FnOnce(&[String], &str) -> Result<Option<String>>,
{
    let prefix = get_log_dir_prefix();
    let log_dir = Path::new(&prefix);

    if !log_dir.exists() {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("Error: Log directory does not exist: {prefix}");
        }
        return Ok(1);
    }

    debug!(log_dir = %prefix, "launching log viewer");

    // Find log files using walkdir
    let log_files = files::list_logs(log_dir);

    if log_files.is_empty() {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("No log files found in: {prefix}");
        }
        return Ok(1);
    }

    // Build display items (full paths as strings)
    let items = build_log_items(&log_files);

    let selected = select_fn(&items, &prefix).context("fzf log selection failed")?;

    if let Some(raw) = selected {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(1);
        }
        // Extract file path from search result (format: filepath:line:content)
        let file_path = extract_file_path(trimmed);
        #[allow(clippy::print_stdout)]
        {
            if Path::new(file_path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"))
            {
                println!("zcat '{file_path}' | less");
            } else {
                println!("less '{file_path}'");
            }
        }
        Ok(0)
    } else {
        debug!("fzf log selection cancelled");
        Ok(1)
    }
}

/// Build display items from the file list (full paths as strings).
fn build_log_items(log_files: &[PathBuf]) -> Vec<String> {
    log_files
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// Launch fzf with log file items.
///
/// Supports two modes via key bindings:
/// - File mode (default): browse log files with preview
/// - Search mode (Ctrl-S): search content via `rg --search-zip`
// NOTEST(infra): requires interactive fzf terminal session
#[cfg_attr(coverage_nightly, coverage(off))]
fn run_fzf_log_selection(items: &[String], log_dir: &str) -> Result<Option<String>> {
    let preview_cmd = "f={}; if [[ $f == *.gz ]]; then zcat \"$f\" 2>/dev/null | head --lines=500; else head --lines=500 \"$f\" 2>/dev/null; fi";

    let find_cmd = format!(
        "find '{log_dir}' -type f \\( -name '*.log' -o -name '*.log.gz' \\) 2>/dev/null | sort --reverse"
    );
    let search_cmd = format!(
        "rg --search-zip --color=always --line-number --no-heading --smart-case -- {{q}} '{log_dir}' 2>/dev/null || true"
    );

    let file_header = "ctrl-s: Search mode | ctrl-j/k: Preview scroll";
    let search_header = "ctrl-f: File mode | Type to search content";

    let bind_search_mode = format!(
        "ctrl-s:change-prompt([Search] > )+disable-search+reload({search_cmd})+rebind(change)+change-preview-window(hidden)+change-header({search_header})"
    );
    let bind_file_mode = format!(
        "ctrl-f:change-prompt([File] > )+enable-search+reload({find_cmd})+unbind(change)+change-preview-window(right:60%:wrap)+change-header({file_header})"
    );
    let bind_on_change = format!("change:reload:sleep 0.1; {search_cmd}");

    let opts = fzf::Options {
        height: String::from("90%"),
        reverse: true,
        exact: true,
        ansi: true,
        prompt: String::from("[File] > "),
        header: Some(String::from(file_header)),
        preview: Some(String::from(preview_cmd)),
        preview_window: Some(String::from("right:60%:wrap")),
        border: Some(String::from("rounded")),
        margin: None,
        delimiter: Some(String::from(":")),
        nth: Some(String::from("1")),
        bind: vec![
            String::from("ctrl-j:preview-down,ctrl-k:preview-up"),
            String::from("ctrl-d:preview-page-down,ctrl-u:preview-page-up"),
            bind_search_mode,
            bind_file_mode,
            bind_on_change,
            String::from("start:unbind(change)"),
        ],
    };

    fzf::run(items, &opts)
}

/// Extract the file path from fzf output.
///
/// In search mode, fzf returns `filepath:line:content`. This function
/// strips the `:line:content` suffix by matching `.log:` or `.log.gz:`.
/// In file mode, the output is already a plain file path.
fn extract_file_path(raw: &str) -> &str {
    // Check longer suffix first to avoid partial match
    for (pattern, ext_len) in [(".log.gz:", 7_usize), (".log:", 4_usize)] {
        if let Some(pos) = raw.find(pattern)
            && let Some(path) = raw.get(..pos.saturating_add(ext_len))
        {
            return path;
        }
    }
    raw
}

/// Get the log directory prefix from the environment or default.
fn get_log_dir_prefix() -> String {
    log_dir::get_prefix()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]
    #![allow(clippy::undocumented_unsafe_blocks)]
    #![allow(clippy::panic)]

    use std::env;
    use std::fs;

    use serial_test::serial;

    use super::*;

    #[test]
    fn build_log_items_creates_full_path_strings() {
        // Arrange
        let files = vec![
            PathBuf::from("/logs/app.log"),
            PathBuf::from("/logs/subdir/deep.log.gz"),
        ];

        // Act
        let items = build_log_items(&files);

        // Assert
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "/logs/app.log");
        assert_eq!(items[1], "/logs/subdir/deep.log.gz");
    }

    #[test]
    fn build_log_items_empty_list() {
        // Arrange
        let files: Vec<PathBuf> = vec![];

        // Act
        let items = build_log_items(&files);

        // Assert
        assert!(items.is_empty());
    }

    /// Helper to save and restore `FTERM_LOG_DIR_PREFIX`.
    fn with_log_dir_prefix<F, R>(value: &str, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let original = env::var("FTERM_LOG_DIR_PREFIX").ok();
        unsafe { env::set_var("FTERM_LOG_DIR_PREFIX", value) };
        let result = f();
        unsafe {
            match &original {
                Some(v) => env::set_var("FTERM_LOG_DIR_PREFIX", v),
                None => env::remove_var("FTERM_LOG_DIR_PREFIX"),
            }
        };
        result
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_no_log_dir_returns_error_1() {
        // Arrange — point to a directory that does not exist
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("nonexistent_sub");
        let path = missing.to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — directory does not exist → returns 1 without creating
            run_inner(|_items, _prefix| panic!("select_fn should not be called"))
        });

        // Assert
        assert_eq!(result.unwrap(), 1);
        assert!(!missing.exists(), "directory should NOT have been created");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_empty_log_dir_returns_1() {
        // Arrange — create an empty temporary directory
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act
            run_inner(|_items, _prefix| panic!("select_fn should not be called"))
        });

        // Assert
        assert_eq!(result.unwrap(), 1);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_selection_returns_less_command_for_log() {
        // Arrange — create a temp dir with a .log file
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("test.log");
        fs::write(&log_path, "some log content").unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — closure returns a selected path
            run_inner(|_items, _prefix| Ok(Some(String::from("/path/to/file.log"))))
        });

        // Assert
        assert_eq!(result.unwrap(), 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_selection_cancelled_returns_1() {
        // Arrange — create a temp dir with a .log file
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("test.log");
        fs::write(&log_path, "some log content").unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — closure returns None (user cancelled)
            run_inner(|_items, _prefix| Ok(None))
        });

        // Assert
        assert_eq!(result.unwrap(), 1);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_selection_empty_string_returns_1() {
        // Arrange — create a temp dir with a .log file
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("test.log");
        fs::write(&log_path, "some log content").unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — closure returns an empty string
            run_inner(|_items, _prefix| Ok(Some(String::new())))
        });

        // Assert
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn extract_file_path_plain_log() {
        // Arrange & Act & Assert — plain file path (file mode)
        assert_eq!(extract_file_path("/logs/app.log"), "/logs/app.log");
    }

    #[test]
    fn extract_file_path_gz_log() {
        // Arrange & Act & Assert — gz file path (file mode)
        assert_eq!(extract_file_path("/logs/app.log.gz"), "/logs/app.log.gz");
    }

    #[test]
    fn extract_file_path_search_result_log() {
        // Arrange & Act & Assert — search result with :line:content
        assert_eq!(
            extract_file_path("/logs/app.log:42:some matched content"),
            "/logs/app.log"
        );
    }

    #[test]
    fn extract_file_path_search_result_gz() {
        // Arrange & Act & Assert — search result from .gz file
        assert_eq!(
            extract_file_path("/logs/app.log.gz:10:matched line"),
            "/logs/app.log.gz"
        );
    }

    #[test]
    fn extract_file_path_unknown_format() {
        // Arrange & Act & Assert — unknown format returns as-is
        assert_eq!(extract_file_path("some random text"), "some random text");
    }

    #[test]
    fn output_format_gz_file() {
        // Arrange
        let path = "/logs/test.log.gz";

        // Act & Assert — .gz files should produce zcat command
        let is_gz = Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"));
        assert!(is_gz);
        let cmd = format!("zcat '{path}' | less");
        assert_eq!(cmd, "zcat '/logs/test.log.gz' | less");
    }

    #[test]
    fn output_format_plain_log_file() {
        // Arrange
        let path = "/logs/test.log";

        // Act & Assert — plain files should produce less command
        let is_gz = Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"));
        assert!(!is_gz);
        let cmd = format!("less '{path}'");
        assert_eq!(cmd, "less '/logs/test.log'");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_selection_returns_zcat_for_gz() {
        // Arrange — create a temp dir with a .log.gz file
        let dir = tempfile::TempDir::new().unwrap();
        let gz_path = dir.path().join("session.log.gz");
        fs::write(&gz_path, "compressed").unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — closure returns a selected .gz path
            run_inner(|_items, _prefix| Ok(Some(String::from("/var/log/session.log.gz"))))
        });

        // Assert — gz files should produce zcat command, returns 0
        assert_eq!(result.unwrap(), 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_selection_from_search_result_log() {
        // Arrange — create a temp dir with a .log file
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("app.log");
        fs::write(&log_path, "content").unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — search result format: filepath:line:content
            run_inner(|_items, _prefix| {
                Ok(Some(String::from("/logs/app.log:42:some matched line")))
            })
        });

        // Assert — path is extracted from search result, returns 0
        assert_eq!(result.unwrap(), 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_selection_from_search_result_gz() {
        // Arrange — create a temp dir with a .log file (any log file to avoid empty check)
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("arch.log");
        fs::write(&log_path, "content").unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — gz search result format: filepath.log.gz:line:content
            run_inner(|_items, _prefix| Ok(Some(String::from("/logs/app.log.gz:10:matched line"))))
        });

        // Assert — gz path extracted, produces zcat command, returns 0
        assert_eq!(result.unwrap(), 0);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    #[serial(env)]
    fn run_inner_select_fn_error_propagates() {
        // Arrange — create a temp dir with a .log file
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("error.log");
        fs::write(&log_path, "content").unwrap();
        let path = dir.path().to_string_lossy().into_owned();

        let result = with_log_dir_prefix(&path, || {
            // Act — closure returns an error
            run_inner(|_items, _prefix| Err(anyhow::anyhow!("fzf crashed")))
        });

        // Assert — error propagates
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("fzf crashed") || err_msg.contains("fzf log selection failed"));
    }
}
