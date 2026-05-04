//! OpenTelemetry instrumentation root module.
//!
//! Hosts cross-signal conventions and the metrics submodule. Exposes
//! `init_otel`, `init_subscriber`, and `shutdown_otel` so that `main`
//! can set up and tear down all three signals (traces / logs / metrics)
//! without knowing provider internals.
//!
//! Add `tracing` / `logs` submodules here when adopting those signals
//! independently.

pub mod metrics;

#[cfg(feature = "otel")]
pub mod conventions;

/// Tuple of optional `OTel` providers returned by [`init_otel`].
///
/// Order: `(tracer_provider, meter_provider, logger_provider)`.
/// Each entry is `None` when `OTEL_EXPORTER_OTLP_ENDPOINT` is unset or
/// when the corresponding exporter fails to build.
#[cfg(feature = "otel")]
pub type OtelProviders = (
    Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
);

/// Unit placeholder used when the `otel` feature is disabled.
#[cfg(not(feature = "otel"))]
pub type OtelProviders = ();

// ── Provider initialisation ──────────────────────────────────────────────────

/// Initialise `OTel` providers and register them as global defaults.
///
/// Reads `OTEL_EXPORTER_OTLP_ENDPOINT` at runtime. Returns `(None, None,
/// None)` (or `()` without the `otel` feature) when the variable is absent
/// or empty so the application degrades gracefully to fmt-only logging.
#[cfg(feature = "otel")]
#[must_use]
pub fn init_otel() -> OtelProviders {
    let has_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .filter(|ep| !ep.is_empty())
        .is_some();

    if !has_endpoint {
        return (None, None, None);
    }

    let resource = build_resource();
    let tracer_provider = build_tracer_provider(resource.clone());
    let logger_provider = build_logger_provider(resource.clone());
    let meter_provider = build_meter_provider(resource);
    (tracer_provider, meter_provider, logger_provider)
}

/// No-op: `OTel` providers are disabled without the `otel` feature.
#[cfg(not(feature = "otel"))]
pub fn init_otel() -> OtelProviders {}

/// Build a `Resource` describing this service instance.
#[cfg(feature = "otel")]
fn build_resource() -> opentelemetry_sdk::Resource {
    use opentelemetry::KeyValue;
    use opentelemetry_semantic_conventions::attribute;

    let service_name =
        std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| String::from(env!("CARGO_PKG_NAME")));

    opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name)
        .with_attributes([
            KeyValue::new(attribute::SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            KeyValue::new(
                attribute::SERVICE_INSTANCE_ID,
                gethostname::gethostname().to_string_lossy().into_owned(),
            ),
            KeyValue::new(attribute::VCS_REF_HEAD_REVISION, env!("GIT_HASH")),
        ])
        .build()
}

/// Build and register a `SdkTracerProvider` with a batch HTTP exporter.
#[cfg(feature = "otel")]
fn build_tracer_provider(
    resource: opentelemetry_sdk::Resource,
) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    use opentelemetry::global;
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};

    let exporter = SpanExporter::builder().with_http().build().ok()?;
    let provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(provider.clone());
    Some(provider)
}

/// Build a `SdkLoggerProvider` with a batch HTTP exporter.
#[cfg(feature = "otel")]
fn build_logger_provider(
    resource: opentelemetry_sdk::Resource,
) -> Option<opentelemetry_sdk::logs::SdkLoggerProvider> {
    use opentelemetry_otlp::LogExporter;
    use opentelemetry_sdk::logs::SdkLoggerProvider;

    let exporter = LogExporter::builder().with_http().build().ok()?;
    Some(
        SdkLoggerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(exporter)
            .build(),
    )
}

/// Build and register a `SdkMeterProvider` with a periodic HTTP exporter.
#[cfg(feature = "otel")]
fn build_meter_provider(
    resource: opentelemetry_sdk::Resource,
) -> Option<opentelemetry_sdk::metrics::SdkMeterProvider> {
    use opentelemetry::global;
    use opentelemetry_otlp::MetricExporter;
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    let exporter = MetricExporter::builder().with_http().build().ok()?;
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(resource)
        .build();

    global::set_meter_provider(provider.clone());
    Some(provider)
}

// ── Subscriber initialisation ────────────────────────────────────────────────

/// Wire up the `tracing` subscriber using the providers from [`init_otel`].
///
/// With the `otel` feature: installs an `EnvFilter` + `fmt` + `OTel` trace
/// layer + `OTel` log bridge. Without it: installs a plain `fmt` subscriber.
/// Must be called before any `tracing::*` macro is used.
#[cfg(feature = "otel")]
pub fn init_subscriber(providers: &OtelProviders) {
    use tracing_subscriber::{filter::EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer();

    let (tracer_provider, _, logger_provider) = providers;

    let otel_trace_layer = tracer_provider.as_ref().map(|p| {
        use opentelemetry::trace::TracerProvider as _;
        let tracer = p.tracer(env!("CARGO_PKG_NAME"));
        tracing_opentelemetry::layer().with_tracer(tracer)
    });

    let otel_log_layer = logger_provider
        .as_ref()
        .map(opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_trace_layer)
        .with(otel_log_layer)
        .init();
}

/// Install a plain `fmt` subscriber (no `OTel`).
#[cfg(not(feature = "otel"))]
pub fn init_subscriber(_providers: &OtelProviders) {
    use tracing_subscriber::filter::EnvFilter;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

// ── Provider shutdown ────────────────────────────────────────────────────────

/// Flush and shut down all `OTel` providers in the correct order.
///
/// Shutdown order: tracer → meter (`force_flush` then `shutdown`) → logger.
/// Keeping the logger last allows tracer and meter shutdown errors to be
/// emitted as `OTel` log records before the logger is torn down.
///
/// # Panics
///
/// Does not panic; shutdown errors are logged as warnings via `tracing`.
#[cfg(feature = "otel")]
pub fn shutdown_otel(providers: OtelProviders) {
    let (tracer_provider, meter_provider, logger_provider) = providers;

    if let Some(p) = tracer_provider
        && let Err(e) = p.shutdown()
    {
        tracing::warn!("OTel tracer shutdown failed: {e}");
    }
    if let Some(p) = meter_provider {
        if let Err(e) = p.force_flush() {
            tracing::warn!("OTel meter flush failed: {e}");
        }
        if let Err(e) = p.shutdown() {
            tracing::warn!("OTel meter shutdown failed: {e}");
        }
    }
    if let Some(p) = logger_provider
        && let Err(e) = p.shutdown()
    {
        tracing::warn!("OTel logger shutdown failed: {e}");
    }
}

/// No-op: `OTel` shutdown is disabled without the `otel` feature.
#[cfg(not(feature = "otel"))]
pub fn shutdown_otel(_providers: OtelProviders) {}
