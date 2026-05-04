#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
//! fterm — SSH/SCP connection management tool.

mod cli;
pub mod command;
pub mod config;
pub mod external;
pub mod logging;
pub mod telemetry;
pub mod tmux;
pub mod util;
pub mod validate;

use std::io;
use std::time::Instant;

use anyhow::Context;
use clap::Parser;

use crate::cli::{Cli, Commands};
use crate::telemetry::metrics::Meters;

// NOTEST(unreachable): process entry point; global init and process::exit are not unit-testable
#[cfg_attr(coverage_nightly, coverage(off))]
fn main() {
    let providers = telemetry::init_otel();
    telemetry::init_subscriber(&providers);

    let meters = Meters::default();

    let exit_code = {
        // Root span wraps all command processing so child spans share a
        // single trace_id. The block scope ensures _guard drops — and
        // therefore the root span ends — before shutdown_otel runs.
        // NOTEST(env): OTel feature gate; compiled only with --features otel
        #[cfg(feature = "otel")]
        let _root = tracing::info_span!("main").entered();

        let cli = Cli::parse();
        let runner = external::RealCommandRunner::new();
        dispatch(cli, &runner, &meters)
    };

    telemetry::shutdown_otel(providers);

    #[allow(clippy::exit)]
    std::process::exit(exit_code);
}

/// Dispatch subcommands, recording execution timing and error metrics.
fn dispatch(cli: Cli, runner: &dyn external::CommandRunner, meters: &Meters) -> i32 {
    match cli.command {
        Commands::Fssh => run_timed(meters, "fssh", || command::fssh::run(runner)),
        Commands::Ssh(args) => run_timed(meters, "ssh", || command::ssh::run(runner, &args.args)),
        Commands::Scp(args) => run_timed(meters, "scp", || command::scp::run(runner, &args.args)),
        Commands::Flog => run_timed(meters, "flog", command::flog::run),
        Commands::Fgen => run_timed(meters, "fgen", command::fgen::run),
        Commands::SshAdd(args) => {
            run_timed(meters, "ssh-add", || command::ssh_add::run(&args.args))
        }
        Commands::SshKeygen(args) => run_timed(meters, "ssh-keygen", || {
            command::ssh_keygen::run(&args.args)
        }),
        Commands::Completion(args) => run_timed(meters, "completion", || {
            command::completion::run(&args.shell, args.list_hosts)
        }),
        Commands::LogFilter => run_timed(meters, "log-filter", || run_log_filter().map(|()| 0)),
    }
}

/// Execute a timed subcommand, record metrics, and return the exit code.
fn run_timed<F>(meters: &Meters, command: &str, f: F) -> i32
where
    F: FnOnce() -> anyhow::Result<i32>,
{
    let start = Instant::now();
    let result = f();
    meters.record_command_duration(command, start.elapsed().as_secs_f64());
    match result {
        Ok(code) => code,
        Err(ref e) => {
            meters.record_command_error(command, error_kind(e));
            tracing::error!("{e:#}");
            1
        }
    }
}

/// Classify an error into a short label for the `fterm.error.kind` attribute.
fn error_kind(err: &anyhow::Error) -> &'static str {
    if err.downcast_ref::<std::io::Error>().is_some() {
        "io"
    } else {
        "unknown"
    }
}

/// Run the log-filter subcommand: read stdin, strip ANSI, prepend timestamps.
fn run_log_filter() -> anyhow::Result<()> {
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    logging::filter::process_stream(stdin, stdout).context("log-filter stream processing failed")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use std::io::Cursor;

    use super::*;

    #[cfg_attr(miri, ignore)]
    #[test]
    fn run_log_filter_processes_empty_stream() {
        // Arrange — empty input; process_stream reads from a Cursor<&[u8]>
        let input: &[u8] = b"";
        let stdin = Cursor::new(input);
        let mut stdout_buf = Vec::new();

        // Act
        let result = logging::filter::process_stream(stdin, &mut stdout_buf);

        // Assert — empty input produces no output and no error
        assert!(result.is_ok());
        assert!(stdout_buf.is_empty());
    }

    #[test]
    fn run_timed_ok_returns_exit_code() {
        // Arrange
        let meters = Meters::default();

        // Act
        let result = run_timed(&meters, "test", || Ok(42));

        // Assert
        assert_eq!(result, 42);
    }

    #[test]
    fn run_timed_err_returns_1() {
        // Arrange
        let meters = Meters::default();

        // Act
        let result = run_timed(&meters, "test", || {
            Err(anyhow::anyhow!("something went wrong"))
        });

        // Assert
        assert_eq!(result, 1);
    }

    #[test]
    fn run_timed_ok_zero_returns_0() {
        // Arrange
        let meters = Meters::default();

        // Act
        let result = run_timed(&meters, "test", || Ok(0));

        // Assert
        assert_eq!(result, 0);
    }

    #[test]
    fn error_kind_io_error() {
        // Arrange
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = anyhow::Error::from(io_err);

        // Act
        let kind = error_kind(&err);

        // Assert
        assert_eq!(kind, "io");
    }

    #[test]
    fn error_kind_unknown_error() {
        // Arrange
        let err = anyhow::anyhow!("some other error");

        // Act
        let kind = error_kind(&err);

        // Assert
        assert_eq!(kind, "unknown");
    }
}
