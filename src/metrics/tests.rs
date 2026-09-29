#![allow(clippy::float_cmp, reason = "the counters are small whole numbers")]

use super::*;

fn sample(families: &[MetricFamily], name: &str, outcome: Option<&str>) -> f64 {
    let family = families
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no family {name}"));
    family
        .samples
        .iter()
        .find(|s| outcome.is_none_or(|o| s.labels == vec![("outcome".to_owned(), o.to_owned())]))
        .unwrap()
        .value
}

#[test]
fn counters_follow_the_calls() {
    let metrics = Metrics::default();
    for _ in 0..5 {
        metrics.started();
    }
    metrics.ended(Outcome::Succeeded);
    metrics.ended(Outcome::Failed);
    metrics.ended(Outcome::Cancelled);
    metrics.ended(Outcome::TimedOut);
    metrics.rows(7);
    let families = metrics.collect();
    assert_eq!(sample(&families, "duckdb_calls_started_total", None), 5.0);
    assert_eq!(
        sample(&families, "duckdb_calls_total", Some("succeeded")),
        1.0
    );
    assert_eq!(sample(&families, "duckdb_calls_total", Some("failed")), 1.0);
    assert_eq!(
        sample(&families, "duckdb_calls_total", Some("cancelled")),
        1.0
    );
    assert_eq!(
        sample(&families, "duckdb_calls_total", Some("timed_out")),
        1.0
    );
    assert_eq!(sample(&families, "duckdb_calls_open", None), 1.0);
    assert_eq!(sample(&families, "duckdb_rows_returned_total", None), 7.0);
}

#[test]
fn the_open_gauge_does_not_go_below_zero() {
    let metrics = Metrics::default();
    metrics.ended(Outcome::Succeeded);
    assert_eq!(sample(&metrics.collect(), "duckdb_calls_open", None), 0.0);
}

#[test]
fn kinds_and_names_follow_the_rules() {
    for family in Metrics::default().collect() {
        assert!(!family.name.starts_with("autumn_"), "{}", family.name);
        assert!(!family.help.is_empty());
        let counter = family.name.ends_with("_total");
        assert_eq!(
            counter,
            matches!(family.kind, MetricKind::Counter),
            "{}",
            family.name
        );
    }
}
