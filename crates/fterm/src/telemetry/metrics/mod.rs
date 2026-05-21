//! Synchronous metric instruments for fterm.
//!
//! Create `Meters` once at startup (after `SdkMeterProvider` is registered
//! globally) and pass by reference to each subcommand. All methods are no-ops
//! when compiled without the `otel` feature.

#[cfg(all(feature = "process-metrics", not(miri)))]
pub mod process;

#[cfg(feature = "otel")]
use opentelemetry::KeyValue;
#[cfg(feature = "otel")]
use opentelemetry::metrics::{Counter, Histogram};

#[cfg(feature = "otel")]
use crate::telemetry::conventions::{attribute as fterm_attr, metric as fterm_metric};

/// Application metric instruments.
///
/// All fields are `#[cfg(feature = "otel")]`-gated; the struct degrades to a
/// zero-sized unit type when `OTel` is disabled so call sites require no `#[cfg]`.
#[cfg(feature = "otel")]
pub struct Meters {
    command_duration: Histogram<f64>,
    command_errors: Counter<u64>,
    #[cfg(all(feature = "process-metrics", not(miri)))]
    _process: process::Handles,
}

#[cfg(feature = "otel")]
impl std::fmt::Debug for Meters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meters").finish_non_exhaustive()
    }
}

/// No-op unit struct used when the `otel` feature is disabled.
#[cfg(not(feature = "otel"))]
#[derive(Default, Debug, Clone, Copy)]
pub struct Meters;

#[cfg(feature = "otel")]
impl Default for Meters {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "otel")]
impl Meters {
    /// Create metric instruments from the global meter provider.
    ///
    /// Must be called after `opentelemetry::global::set_meter_provider` has
    /// been invoked; calling beforehand produces no-op instruments.
    #[must_use]
    pub fn new() -> Self {
        Self::from_meter(&opentelemetry::global::meter(env!("CARGO_PKG_NAME")))
    }

    fn from_meter(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            command_duration: meter
                .f64_histogram(fterm_metric::COMMAND_DURATION)
                .with_unit("s")
                .with_description("End-to-end subcommand execution latency")
                .build(),
            command_errors: meter
                .u64_counter(fterm_metric::COMMAND_ERRORS)
                .with_unit("{error}")
                .with_description("Subcommand invocations that resulted in an error")
                .build(),
            #[cfg(all(feature = "process-metrics", not(miri)))]
            _process: process::Handles::register(meter),
        }
    }

    /// Record the execution duration of a subcommand.
    pub fn record_command_duration(&self, command: &str, duration_s: f64) {
        self.command_duration.record(
            duration_s,
            &[KeyValue::new(fterm_attr::COMMAND, command.to_owned())],
        );
    }

    /// Record a subcommand error with an error-kind label.
    pub fn record_command_error(&self, command: &str, kind: &str) {
        self.command_errors.add(
            1,
            &[
                KeyValue::new(fterm_attr::COMMAND, command.to_owned()),
                KeyValue::new(fterm_attr::ERROR_KIND, kind.to_owned()),
            ],
        );
    }
}

#[cfg(not(feature = "otel"))]
impl Meters {
    /// No-op: metrics are disabled without the `otel` feature.
    pub fn record_command_duration(&self, _command: &str, _duration_s: f64) {}

    /// No-op: metrics are disabled without the `otel` feature.
    pub fn record_command_error(&self, _command: &str, _kind: &str) {}
}

#[cfg(all(test, feature = "otel"))]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::redundant_closure_for_method_calls)]

    use std::time::Duration;

    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
        data::{AggregatedMetrics, MetricData},
    };

    use super::*;

    /// Build a test `SdkMeterProvider` backed by an in-memory exporter.
    ///
    /// Uses a 1-hour `PeriodicReader` interval so the background collection
    /// thread never fires during tests; only `force_flush` triggers an export.
    /// Meters are created directly from the provider — no global state is used.
    fn test_provider() -> (SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone())
            .with_interval(Duration::from_hours(1))
            .build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        (provider, exporter)
    }

    /// Find a metric by name in the collected `ResourceMetrics`.
    fn find_metric<'a>(
        metrics: &'a [opentelemetry_sdk::metrics::data::ResourceMetrics],
        name: &str,
    ) -> Option<&'a opentelemetry_sdk::metrics::data::Metric> {
        metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .find(|m| m.name() == name)
    }

    #[test]
    fn record_command_duration_adds_histogram_data_point() {
        // Arrange — create Meters directly from provider; no global state
        let (provider, exporter) = test_provider();
        let meters = Meters::from_meter(&provider.meter(env!("CARGO_PKG_NAME")));

        // Act
        meters.record_command_duration("flog", 0.123);
        provider.force_flush().expect("flush failed");

        // Assert
        let resource_metrics = exporter.get_finished_metrics().expect("no data");
        let metric =
            find_metric(&resource_metrics, fterm_metric::COMMAND_DURATION).expect("metric missing");

        let count = match metric.data() {
            AggregatedMetrics::F64(MetricData::Histogram(hist)) => {
                hist.data_points().map(|dp| dp.count()).sum::<u64>()
            }
            _ => panic!("unexpected metric data type"),
        };
        assert_eq!(count, 1, "expected one histogram data point");
    }

    #[test]
    fn record_command_error_increments_counter() {
        // Arrange — create Meters directly from provider; no global state
        let (provider, exporter) = test_provider();
        let meters = Meters::from_meter(&provider.meter(env!("CARGO_PKG_NAME")));

        // Act
        meters.record_command_error("ssh", "io");
        meters.record_command_error("ssh", "io");
        provider.force_flush().expect("flush failed");

        // Assert
        let resource_metrics = exporter.get_finished_metrics().expect("no data");
        let metric =
            find_metric(&resource_metrics, fterm_metric::COMMAND_ERRORS).expect("metric missing");

        let total: u64 = match metric.data() {
            AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                sum.data_points().map(|dp| dp.value()).sum()
            }
            _ => panic!("unexpected metric data type"),
        };
        assert_eq!(total, 2, "expected two error increments");
    }
}
