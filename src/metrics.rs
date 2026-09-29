//! Call counters and the Autumn metrics source.
//!
//! # Contract
//!
//! - `duckdb_calls_started_total` counts the calls that got past the shutdown check.
//! - `duckdb_calls_total` counts the calls that ended, with the label `outcome`.
//! - `duckdb_calls_open` is the calls that did not end yet.
//! - `duckdb_rows_returned_total` counts the rows that queries gave to callers.

use std::sync::atomic::{AtomicU64, Ordering};

use autumn_web::actuator::{MetricFamily, MetricKind, MetricSample, MetricsSource};

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

/// Counters for all calls of one app.
#[derive(Debug, Default)]
pub(crate) struct Metrics {
    started: AtomicU64,
    succeeded: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    timed_out: AtomicU64,
    rows: AtomicU64,
    open: AtomicU64,
}

impl Metrics {
    pub(crate) fn started(&self) {
        self.started.fetch_add(1, Ordering::Relaxed);
        self.open.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn ended(&self, outcome: Outcome) {
        let counter = match outcome {
            Outcome::Succeeded => &self.succeeded,
            Outcome::Failed => &self.failed,
            Outcome::Cancelled => &self.cancelled,
            Outcome::TimedOut => &self.timed_out,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        let _ = self
            .open
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(1))
            });
    }

    pub(crate) fn rows(&self, count: usize) {
        let count = u64::try_from(count).unwrap_or(u64::MAX);
        self.rows.fetch_add(count, Ordering::Relaxed);
    }

    fn load(counter: &AtomicU64) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "Prometheus values are f64")]
        let value = counter.load(Ordering::Relaxed) as f64;
        value
    }

    fn single(name: &str, help: &str, kind: MetricKind, counter: &AtomicU64) -> MetricFamily {
        MetricFamily {
            name: name.to_owned(),
            help: help.to_owned(),
            kind,
            samples: vec![MetricSample {
                labels: Vec::new(),
                value: Self::load(counter),
            }],
        }
    }
}

impl MetricsSource for Metrics {
    fn collect(&self) -> Vec<MetricFamily> {
        let outcome = |name: &str, counter: &AtomicU64| MetricSample {
            labels: vec![("outcome".to_owned(), name.to_owned())],
            value: Self::load(counter),
        };
        vec![
            Self::single(
                "duckdb_calls_started_total",
                "DuckDB calls that the plugin started.",
                MetricKind::Counter,
                &self.started,
            ),
            MetricFamily {
                name: "duckdb_calls_total".to_owned(),
                help: "DuckDB calls that ended, by outcome.".to_owned(),
                kind: MetricKind::Counter,
                samples: vec![
                    outcome("succeeded", &self.succeeded),
                    outcome("failed", &self.failed),
                    outcome("cancelled", &self.cancelled),
                    outcome("timed_out", &self.timed_out),
                ],
            },
            Self::single(
                "duckdb_calls_open",
                "DuckDB calls that did not end yet.",
                MetricKind::Gauge,
                &self.open,
            ),
            Self::single(
                "duckdb_rows_returned_total",
                "Rows that DuckDB queries gave to callers.",
                MetricKind::Counter,
                &self.rows,
            ),
        ]
    }
}

#[cfg(test)]
mod tests;
