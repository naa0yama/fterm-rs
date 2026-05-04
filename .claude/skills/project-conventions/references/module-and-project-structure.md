# Module & Project Structure — fterm

> **Shared patterns**: See `~/.claude/skills/rust-project-conventions/references/module-structure.md`
> for visibility rules, mod.rs re-export pattern, size limits, CLI design, and clippy configuration.

## Project Source Layout

```
crates/
  fterm/               # Binary crate — CLI entry point and subcommand dispatch
    src/
      main.rs          # Entry point (OTel init, clap derive, subcommand routing)
      cli.rs           # Top-level clap CLI definition
      command/         # Subcommand implementations (ssh, scp, fssh, flog, fgen, ...)
      telemetry/       # OTel setup (init_otel, init_subscriber, shutdown_otel)
      config.rs        # Application config loading
      external.rs      # External binary detection (fzf, tmux, etc.)
    tests/
      integration_test.rs
  fterm-core/          # Library — core traits, types, pure functions (Miri-safe)
    src/
      lib.rs
      runner.rs        # CommandRunner trait + MockCommandRunner
      check_types.rs   # Validation result types
      ssh_parse.rs     # SSH config primitives
      util/            # Pure utility functions
  fterm-session/       # Library — SSH session management
    src/
      lib.rs
      tmux/            # tmux session, pane, window management
      logging/         # Log file creation, pipe-pane setup/teardown
      util/            # File utilities
  fterm-ssh-config/    # Library — SSH config parsing and validation
    src/
      lib.rs
      config/          # Host blocks, includes, agent keys, connection info
      validate/        # 8 validation checks (syntax, duplicates, identity, ...)
ast-rules/
  *.yml                # Custom ast-grep lint rules
tmp/
  .ssh/                # SSH config test fixtures (git-tracked via !tmp/.ssh/)
```

## Dependency Graph

```
fterm (binary)
  ├── fterm-core
  ├── fterm-session → fterm-core
  └── fterm-ssh-config → fterm-core
```

## OTel / Tracing Setup

- OTel is opt-in (`default = []` in fterm crate).
- Set `OTEL_EXPORTER_OTLP_ENDPOINT` env var to activate OTLP export.
- Without the env var (or empty), only the `fmt` layer is active.
- Build with OTel: `cargo build --features otel`.
- Test tasks automatically set `OTEL_EXPORTER_OTLP_ENDPOINT=""` to prevent OTel panics.
- Feature flags in `crates/fterm/Cargo.toml`:
  ```toml
  [features]
  default = []
  otel = [
  	"dep:gethostname",
  	"dep:opentelemetry",
  	"dep:opentelemetry_sdk",
  	"dep:opentelemetry-otlp",
  	"dep:tracing-opentelemetry",
  	"dep:opentelemetry-appender-tracing",
  	"dep:opentelemetry-semantic-conventions",
  ]
  # Collects OTel-semconv process metrics. Requires `otel`. Disable with --no-default-features.
  process-metrics = [
  	"otel",
  	"dep:sysinfo",
  ]
  ```
- `service.instance.id` is set to `gethostname::gethostname()` (CLI: one instance per host).
- `TraceContextPropagator` and `global::set_tracer_provider()` are set at provider init.
- Transport: HTTP/proto (`http-proto` + `reqwest-blocking-client`), port 4318.
