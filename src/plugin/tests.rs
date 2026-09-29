use std::time::Duration;

use super::*;

#[test]
fn build_declares_the_config_section() {
    let app = autumn_web::app().plugin(DuckDbPlugin::new().config_section("analytics"));
    assert!(app.has_config_section("analytics"));
    assert!(!app.has_config_section("duckdb"));
}

#[test]
fn an_explicit_config_declares_no_section() {
    let app = autumn_web::app().plugin(DuckDbPlugin::new().config(DuckDbConfig::default()));
    assert!(!app.has_config_section("duckdb"));
}

#[test]
fn resolve_applies_changes_then_validates() {
    let source = Source::Explicit(Box::new(DuckDbConfig::default()));
    let config = DuckDbPlugin::resolve(&source, vec![Box::new(|c: &mut DuckDbConfig| c.max_rows = 3)])
        .unwrap();
    assert_eq!(config.max_rows, 3);
    let err = DuckDbPlugin::resolve(&source, vec![Box::new(|c: &mut DuckDbConfig| c.max_rows = 0)])
        .unwrap_err();
    assert!(err.to_string().contains("duckdb.max_rows"), "{err}");
}

#[test]
fn a_bad_section_names_the_section() {
    let source = Source::Section("analytics".to_owned());
    let err = DuckDbPlugin::resolve(&source, vec![Box::new(|c: &mut DuckDbConfig| c.max_rows = 0)])
        .unwrap_err();
    assert!(err.to_string().contains("analytics.max_rows"), "{err}");
}

#[tokio::test]
async fn shutdown_interrupts_open_calls() {
    let shared = Shared::default();
    let db = DuckDb::open(DuckDbConfig::default()).await.unwrap();
    assert!(shared.handle.set(db.clone()).is_ok());
    let task = tokio::spawn(async move {
        db.query("SELECT count(*) FROM range(100000) a, range(10000000) b")
            .fetch()
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    shared.shutdown().await;
    assert_eq!(task.await.unwrap().unwrap_err(), DuckDbError::ShuttingDown);
}

#[tokio::test]
async fn shutdown_before_startup_does_nothing() {
    Shared::default().shutdown().await;
}

#[test]
fn debug_shows_the_config_source() {
    let text = format!(
        "{:?}",
        DuckDbPlugin::new()
            .config_section("analytics")
            .setup(|_| Ok(()))
    );
    assert!(text.contains("analytics"), "{text}");
    assert!(text.contains("setups: 1"), "{text}");
}
