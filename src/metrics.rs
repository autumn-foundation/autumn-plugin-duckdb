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
    pub(crate) fn started(&self) {}

    pub(crate) fn ended(&self, outcome: Outcome) {
        let _ = outcome;
    }

    pub(crate) fn rows(&self, count: usize) {
        let _ = count;
    }
}

impl MetricsSource for Metrics {
    fn collect(&self) -> Vec<MetricFamily> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests;
