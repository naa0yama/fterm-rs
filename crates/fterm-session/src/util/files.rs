//! Log file utilities: listing, compression, and reading.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tracing::{debug, warn};
use walkdir::WalkDir;

use fterm_core::runner::CommandRunner;

/// List log files (`.log`, `.log.gz`) under a directory, sorted by mtime descending.
///
/// Returns an empty vec if the directory does not exist or contains no log files.
pub fn list_logs(dir: &Path) -> Vec<PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }

    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let name = e.file_name().to_string_lossy();
            name.ends_with(".log") || name.ends_with(".log.gz")
        })
        .filter_map(|e| {
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((e.into_path(), mtime))
        })
        .collect();

    // Sort by mtime descending (newest first)
    entries.sort_by(|a, b| b.1.cmp(&a.1));

    entries.into_iter().map(|(path, _)| path).collect()
}

/// Compress all uncompressed `.log` files under a directory with gzip.
///
/// Each `.log` file is compressed to `.log.gz` and the original is removed.
/// Files that are already `.log.gz` are skipped.
///
/// Active log files (those currently written to by another fterm session) are
/// excluded by querying tmux for all pane `@fterm_logging` option values. A
/// compression failure is downgraded to a warning to handle races gracefully.
///
/// # Errors
///
/// Returns an error if the directory cannot be walked.
pub fn compress_logs(runner: &dyn CommandRunner, dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    // Build the set of log paths currently in use by other fterm sessions.
    let active_set: HashSet<PathBuf> = match runner.run(
        "tmux",
        &["list-panes", "-a", "-s", "-F", "#{@fterm_logging}"],
        5,
    ) {
        Err(e) => {
            warn!(
                ?e,
                "failed to query tmux for active log paths; skipping active-log protection"
            );
            HashSet::new()
        }
        Ok(output) if output.exit_code != 0 => {
            debug!(
                exit_code = output.exit_code,
                "tmux list-panes returned non-zero; assuming no active logs"
            );
            HashSet::new()
        }
        Ok(output) => output
            .stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect(),
    };

    let log_files: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".log"))
        .map(walkdir::DirEntry::into_path)
        .collect();

    for path in &log_files {
        if active_set.contains(path) {
            debug!(
                path = %path.display(),
                "skipping active log (in use by another fterm session)"
            );
            continue;
        }
        if let Err(e) = compress_single_file(path) {
            warn!(
                ?e,
                path = %path.display(),
                "failed to compress log; another fssh may have raced"
            );
        }
    }

    debug!(total = log_files.len(), "compress_logs scan complete");
    Ok(())
}

/// Compress a single file to `.gz` and remove the original.
fn compress_single_file(path: &Path) -> Result<()> {
    let gz_path = PathBuf::from(format!("{}.gz", path.display()));

    let in_file =
        File::open(path).with_context(|| format!("failed to open: {}", path.display()))?;
    let out_file = File::create(&gz_path)
        .with_context(|| format!("failed to create: {}", gz_path.display()))?;
    let mut encoder = GzEncoder::new(out_file, Compression::default());
    io::copy(&mut BufReader::new(in_file), &mut encoder)
        .context("failed to write compressed data")?;
    encoder.finish().context("failed to finish gzip encoding")?;

    fs::remove_file(path).with_context(|| format!("failed to remove: {}", path.display()))?;

    debug!(path = %path.display(), "compressed log file");
    Ok(())
}

/// Read first N lines from a gzip file.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or decompressed.
pub fn read_gz_head(path: &Path, n: usize) -> Result<String> {
    let file = File::open(path).with_context(|| format!("failed to open: {}", path.display()))?;
    read_n_lines(BufReader::new(GzDecoder::new(file)), n, path)
}

/// Read first N lines from a text file.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
pub fn read_head(path: &Path, n: usize) -> Result<String> {
    let file = File::open(path).with_context(|| format!("failed to open: {}", path.display()))?;
    read_n_lines(BufReader::new(file), n, path)
}

fn read_n_lines(reader: impl BufRead, n: usize, path: &Path) -> Result<String> {
    let mut result = String::new();
    for line in reader.lines().take(n) {
        let line = line.with_context(|| format!("failed to read line from: {}", path.display()))?;
        result.push_str(&line);
        result.push('\n');
    }
    Ok(result)
}

/// Check if a path has a `.gz` extension.
#[must_use]
pub fn is_gz(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "gz")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::fs;
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;
    use fterm_core::runner::CommandOutput;
    use fterm_core::runner::MockCommandRunner;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn list_logs_finds_log_and_gz() {
        // Arrange
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("app.log"), "line1\n").unwrap();
        fs::write(dir.path().join("app.log.gz"), "compressed").unwrap();
        fs::write(dir.path().join("readme.txt"), "not a log").unwrap();

        // Act
        let files = list_logs(dir.path());

        // Assert
        assert_eq!(files.len(), 2);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&String::from("app.log")));
        assert!(names.contains(&String::from("app.log.gz")));
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn list_logs_returns_empty_for_missing_dir() {
        // Act
        let files = list_logs(Path::new("/nonexistent/path"));

        // Assert
        assert!(files.is_empty());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn list_logs_finds_in_subdirectories() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("deep.log"), "deep\n").unwrap();

        // Act
        let files = list_logs(dir.path());

        // Assert
        assert_eq!(files.len(), 1);
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn compress_logs_compresses_and_removes_original() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("test.log");
        fs::write(&log_path, "hello world\n").unwrap();
        let runner = MockCommandRunner::new();

        // Act
        compress_logs(&runner, dir.path()).unwrap();

        // Assert
        assert!(!log_path.exists(), "original .log should be removed");
        let gz_path = dir.path().join("test.log.gz");
        assert!(gz_path.exists(), ".log.gz should be created");

        // Verify content is valid gzip
        let content = read_gz_head(&gz_path, 10).unwrap();
        assert_eq!(content, "hello world\n");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn compress_logs_skips_nonexistent_dir() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act & Assert
        assert!(compress_logs(&runner, Path::new("/nonexistent")).is_ok());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn compress_logs_skips_already_compressed() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let gz_path = dir.path().join("already.log.gz");
        fs::write(&gz_path, "fake gz data").unwrap();
        let runner = MockCommandRunner::new();

        // Act
        compress_logs(&runner, dir.path()).unwrap();

        // Assert — file should be unchanged (not double-compressed)
        let content = fs::read_to_string(&gz_path).unwrap();
        assert_eq!(content, "fake gz data");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn compress_logs_skips_active_log_reported_by_tmux() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let active_log = dir.path().join("active.log");
        let old_log = dir.path().join("old.log");
        fs::write(&active_log, "in use\n").unwrap();
        fs::write(&old_log, "old\n").unwrap();

        // Mock tmux to report active_log as in use.
        let runner = MockCommandRunner::new().with_run_response(
            "tmux list-panes -a -s -F #{@fterm_logging}",
            CommandOutput {
                exit_code: 0,
                stdout: format!("{}\n", active_log.display()),
                stderr: String::new(),
            },
        );

        // Act
        compress_logs(&runner, dir.path()).unwrap();

        // Assert — active log must not be compressed, old log must be compressed.
        assert!(active_log.exists(), "active log should not be touched");
        assert!(!old_log.exists(), "old log should be compressed");
        assert!(dir.path().join("old.log.gz").exists());
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn read_gz_head_reads_n_lines() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let gz_path = dir.path().join("test.log.gz");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(b"line1\nline2\nline3\nline4\nline5\n")
            .unwrap();
        let compressed = encoder.finish().unwrap();
        fs::write(&gz_path, &compressed).unwrap();

        // Act
        let result = read_gz_head(&gz_path, 3).unwrap();

        // Assert
        assert_eq!(result, "line1\nline2\nline3\n");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn read_head_reads_n_lines() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.log");
        fs::write(&path, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        // Act
        let result = read_head(&path, 2).unwrap();

        // Assert
        assert_eq!(result, "line1\nline2\n");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn read_head_handles_short_file() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("short.log");
        fs::write(&path, "only one\n").unwrap();

        // Act
        let result = read_head(&path, 100).unwrap();

        // Assert
        assert_eq!(result, "only one\n");
    }

    #[test]
    fn is_gz_detects_gz_extension() {
        assert!(is_gz(Path::new("file.log.gz")));
        assert!(!is_gz(Path::new("file.log")));
        assert!(!is_gz(Path::new("file")));
    }
}
