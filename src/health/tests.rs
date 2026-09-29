use autumn_web::actuator::{HealthIndicator, HealthStatus};

use super::*;
use crate::client::DuckDb;
use crate::config::DuckDbConfig;

async fn started(config: DuckDbConfig) -> (Arc<Shared>, DuckDb) {
    let shared = Arc::new(Shared::default());
    let db = DuckDb::open(config).await.unwrap();
    assert!(shared.handle.set(db.clone()).is_ok());
    (shared, db)
}

#[tokio::test]
async fn a_check_before_startup_is_down() {
    let output = DatabaseCheck::new(Arc::new(Shared::default()))
        .check()
        .await;
    assert_eq!(output.status, HealthStatus::Down);
    assert_eq!(output.details["state"], "not started");
}

#[tokio::test]
async fn a_started_database_is_up() {
    let (shared, _db) = started(DuckDbConfig::default()).await;
    let output = DatabaseCheck::new(shared).check().await;
    assert_eq!(output.status, HealthStatus::Up);
    assert_eq!(output.details["database"], "in-memory");
}

#[tokio::test]
async fn a_file_database_says_file_and_not_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret-name.duckdb");
    let config = DuckDbConfig {
        path: path.to_str().unwrap().to_owned(),
        ..DuckDbConfig::default()
    };
    let (shared, _db) = started(config).await;
    let output = DatabaseCheck::new(shared).check().await;
    assert_eq!(output.details["database"], "file");
    assert!(!format!("{:?}", output.details).contains("secret-name"));
}

#[tokio::test]
async fn a_check_after_shutdown_is_down() {
    let (shared, db) = started(DuckDbConfig::default()).await;
    db.shutdown().await;
    let output = DatabaseCheck::new(shared).check().await;
    assert_eq!(output.status, HealthStatus::Down);
}
