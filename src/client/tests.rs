#![allow(
    clippy::field_reassign_with_default,
    reason = "each test changes one key of the defaults"
)]
#![allow(clippy::float_cmp, reason = "the counters are small whole numbers")]

use std::time::{Duration, Instant};

use serde::Deserialize;

use super::*;
use crate::metrics::Outcome;
use crate::value::Value;
use autumn_web::actuator::MetricsSource;

/// A query that runs for minutes unless DuckDB interrupts it.
const SLOW: &str = "SELECT count(*) FROM range(100000) a, range(10000000) b";

async fn db() -> DuckDb {
    DuckDb::open(DuckDbConfig::default()).await.unwrap()
}

async fn db_with(change: impl FnOnce(&mut DuckDbConfig)) -> DuckDb {
    let mut config = DuckDbConfig::default();
    change(&mut config);
    DuckDb::open(config).await.unwrap()
}

/// Waits until no lease is out. DuckDB must stop the interrupted query for this.
async fn wait_quiet(db: &DuckDb) {
    let start = Instant::now();
    while !db.inner.pool.is_quiet() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the query did not stop"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn counter(db: &DuckDb, name: &str, outcome: Option<&str>) -> f64 {
    db.inner
        .metrics
        .collect()
        .into_iter()
        .find(|f| f.name == name)
        .unwrap()
        .samples
        .into_iter()
        .find(|s| outcome.is_none_or(|o| s.labels[0].1 == o))
        .unwrap()
        .value
}

#[derive(Debug, Deserialize, PartialEq)]
struct Item {
    id: i64,
    name: String,
}

async fn with_items(db: &DuckDb) {
    db.query("CREATE TABLE items (id BIGINT, name VARCHAR)")
        .execute()
        .await
        .unwrap();
    let count = db
        .query("INSERT INTO items VALUES (1, 'a'), (2, 'b'), (3, ?)")
        .bind("it's")
        .execute()
        .await
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn fetch_gives_rows_with_column_names() {
    let db = db().await;
    let rows = db
        .query("SELECT 1 AS id, 'x' AS name UNION ALL SELECT 2, NULL ORDER BY id")
        .fetch()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].columns(), ["id", "name"]);
    assert_eq!(rows[0].get("name"), Some(&Value::Text("x".into())));
    assert_eq!(rows[1].get("name"), Some(&Value::Null));
}

#[tokio::test]
async fn an_empty_result_keeps_no_rows() {
    let db = db().await;
    let rows = db.query("SELECT 1 WHERE false").fetch().await.unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn clones_share_the_database() {
    let db = db().await;
    with_items(&db.clone()).await;
    let items: Vec<Item> = db
        .query("SELECT id, name FROM items WHERE id < ? ORDER BY id")
        .bind(3)
        .fetch_as()
        .await
        .unwrap();
    assert_eq!(
        items,
        vec![
            Item {
                id: 1,
                name: "a".into()
            },
            Item {
                id: 2,
                name: "b".into()
            }
        ]
    );
}

#[tokio::test]
async fn bound_values_are_never_sql() {
    let db = db().await;
    with_items(&db).await;
    let name: String = db
        .query("SELECT name FROM items WHERE id = $1")
        .bind(3)
        .fetch_one_as()
        .await
        .unwrap();
    assert_eq!(name, "it's");
    let none: Option<Item> = db
        .query("SELECT id, name FROM items WHERE name = ?")
        .bind("x' OR '1'='1")
        .fetch_optional_as()
        .await
        .unwrap();
    assert_eq!(none, None);
}

#[tokio::test]
async fn fetch_optional_gives_the_first_row() {
    let db = db().await;
    with_items(&db).await;
    let row = db
        .query("SELECT id FROM items ORDER BY id")
        .fetch_optional()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get("id"), Some(&Value::Int(1)));
    let none = db
        .query("SELECT id FROM items WHERE id > 9")
        .fetch_optional()
        .await
        .unwrap();
    assert!(none.is_none());
}

#[tokio::test]
async fn fetch_one_gives_not_found_for_no_rows() {
    let db = db().await;
    let err = db
        .query("SELECT 1 WHERE false")
        .fetch_one_as::<i64>()
        .await
        .unwrap_err();
    assert_eq!(err, DuckDbError::NotFound);
}

#[tokio::test]
async fn a_row_that_does_not_decode_gives_a_decode_error() {
    let db = db().await;
    let err = db
        .query("SELECT 'x' AS id, 'y' AS name")
        .fetch_as::<Item>()
        .await
        .unwrap_err();
    assert!(matches!(err, DuckDbError::Decode(_)), "{err:?}");
}

#[tokio::test]
async fn more_than_one_statement_is_refused_before_it_runs() {
    let db = db().await;
    let err = db
        .query("CREATE TABLE side_effect (x INT); SELECT 1")
        .fetch()
        .await
        .unwrap_err();
    assert!(
        matches!(err, DuckDbError::MultipleStatements { statements: 2 }),
        "{err:?}"
    );
    let err = db
        .query("CREATE TABLE side_effect (x INT); SELECT 1")
        .execute()
        .await
        .unwrap_err();
    assert!(
        matches!(err, DuckDbError::MultipleStatements { .. }),
        "{err:?}"
    );
    let tables: i64 = db
        .query("SELECT count(*) FROM duckdb_tables() WHERE table_name = 'side_effect'")
        .fetch_one_as()
        .await
        .unwrap();
    assert_eq!(tables, 0);
}

#[tokio::test]
async fn lexer_tricks_do_not_run_a_second_statement() {
    let db = db().await;
    for sql in [
        "SELECT 1 --\r; CREATE TABLE marker (i INT); SELECT 2",
        "SELECT 1 AS a$x$; CREATE TABLE marker (i INT); SELECT 1 AS b$x$",
        "SELECT $é$'$é$; CREATE TABLE marker (i INT); SELECT '1'",
    ] {
        let err = db.query(sql).execute().await.unwrap_err();
        assert!(
            matches!(err, DuckDbError::MultipleStatements { .. }),
            "{sql}: {err:?}"
        );
    }
    let err = db
        .query("SELECT 'open; CREATE TABLE marker (i INT)")
        .execute()
        .await
        .unwrap_err();
    assert_eq!(err, DuckDbError::OpenLiteral);
    let tables: i64 = db
        .query("SELECT count(*) FROM duckdb_tables() WHERE table_name = 'marker'")
        .fetch_one_as()
        .await
        .unwrap();
    assert_eq!(tables, 0);
}

#[tokio::test]
async fn session_state_never_reaches_the_next_call() {
    let db = db_with(|c| c.max_connections = 1).await;
    db.with_connection(|conn| {
        conn.execute_batch(
            "CREATE TABLE t AS SELECT 'main' AS v;
             CREATE SCHEMA other;
             CREATE TABLE other.t AS SELECT 'other' AS v;",
        )
    })
    .await
    .unwrap();
    db.with_connection(|conn| {
        conn.execute_batch(
            "CREATE TEMP TABLE secret AS SELECT 42 AS n;
             SET VARIABLE v = 7;
             PREPARE p AS SELECT 99;
             USE memory.other;
             BEGIN;
             INSERT INTO main.t VALUES ('open');",
        )
    })
    .await
    .unwrap();
    let v: String = db.query("SELECT v FROM t").fetch_one_as().await.unwrap();
    assert_eq!(v, "main");
    let err = db.query("SELECT * FROM secret").fetch().await.unwrap_err();
    assert_eq!(err.class(), Some("Catalog"));
    let variable: Option<i64> = db
        .query("SELECT getvariable('v')")
        .fetch_one_as()
        .await
        .unwrap();
    assert_eq!(variable, None);
    assert!(db.query("EXECUTE p").fetch().await.is_err());
    let rows: i64 = db
        .query("SELECT count(*) FROM t")
        .fetch_one_as()
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn a_wrong_parameter_count_is_refused() {
    let db = db().await;
    let err = db.query("SELECT ?, ?").bind(1).fetch().await.unwrap_err();
    assert!(
        matches!(
            err,
            DuckDbError::ParameterCount {
                placeholders: 2,
                parameters: 1
            }
        ),
        "{err:?}"
    );
    let err = db.query("SELECT 1").bind(1).execute().await.unwrap_err();
    assert!(matches!(err, DuckDbError::ParameterCount { .. }), "{err:?}");
}

#[tokio::test]
async fn a_sql_error_keeps_the_class() {
    let db = db().await;
    let err = db.query("SELECT * FROM nothing").fetch().await.unwrap_err();
    assert_eq!(err.class(), Some("Catalog"));
    assert_eq!(counter(&db, "duckdb_calls_total", Some("failed")), 1.0);
}

#[tokio::test]
async fn the_row_limit_gives_an_error() {
    let db = db_with(|c| c.max_rows = 2).await;
    assert_eq!(
        db.query("SELECT * FROM range(2)")
            .fetch()
            .await
            .unwrap()
            .len(),
        2
    );
    let err = db
        .query("SELECT * FROM range(3)")
        .fetch()
        .await
        .unwrap_err();
    assert_eq!(err, DuckDbError::TooManyRows { limit: 2 });
}

#[tokio::test]
async fn the_byte_limit_gives_an_error() {
    // A text value counts 16 bytes and its text.
    let db = db_with(|c| c.max_result_bytes = 26).await;
    db.query("SELECT repeat('x', 10)").fetch().await.unwrap();
    let err = db
        .query("SELECT repeat('x', 11)")
        .fetch_optional()
        .await
        .unwrap_err();
    assert_eq!(err, DuckDbError::ResultTooLarge { limit_bytes: 26 });
}

#[tokio::test]
async fn the_byte_limit_counts_all_rows() {
    let db = db_with(|c| c.max_result_bytes = 40).await;
    db.query("SELECT 'xx' FROM range(2)").fetch().await.unwrap();
    let err = db
        .query("SELECT 'xx' FROM range(3)")
        .fetch()
        .await
        .unwrap_err();
    assert_eq!(err, DuckDbError::ResultTooLarge { limit_bytes: 40 });
}

#[tokio::test]
async fn the_byte_limit_counts_empty_items() {
    let db = db_with(|c| c.max_result_bytes = 1000).await;
    let err = db
        .query("SELECT list_transform(range(100000), x -> '') AS l")
        .fetch()
        .await
        .unwrap_err();
    assert_eq!(err, DuckDbError::ResultTooLarge { limit_bytes: 1000 });
}

#[test]
fn reading_rows_stops_after_a_cancel() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let limits = Limits {
        take: usize::MAX,
        max_rows: usize::MAX,
        max_bytes: usize::MAX,
    };
    let err = read_rows(&conn, "SELECT * FROM range(10)", &[], limits, &|| true).unwrap_err();
    assert_eq!(err, DuckDbError::Cancelled);
    let rows = read_rows(&conn, "SELECT * FROM range(10)", &[], limits, &|| false).unwrap();
    assert_eq!(rows.len(), 10);
}

#[tokio::test]
async fn a_timeout_interrupts_the_query() {
    let db = db_with(|c| {
        c.timeout_ms = 200;
        c.max_connections = 1;
    })
    .await;
    let start = Instant::now();
    let err = db.query(SLOW).fetch().await.unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(
        err,
        DuckDbError::Timeout {
            timeout: Duration::from_millis(200)
        }
    );
    wait_quiet(&db).await;
    assert_eq!(counter(&db, "duckdb_calls_total", Some("timed_out")), 1.0);
}

#[tokio::test]
async fn an_interrupt_never_reaches_the_next_call() {
    let db = db_with(|c| {
        c.timeout_ms = 100;
        c.max_connections = 1;
    })
    .await;
    assert!(db.query(SLOW).fetch().await.is_err());
    for _ in 0..50 {
        db.query("SELECT 1").fetch().await.unwrap();
    }
}

#[tokio::test]
async fn an_interrupt_before_the_query_starts_repeats() {
    let db = db_with(|c| c.timeout_ms = 100).await;
    let err = db
        .with_connection(|conn| {
            // DuckDB ignores the first interrupts: no query runs yet.
            std::thread::sleep(Duration::from_millis(300));
            conn.execute_batch(SLOW)
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DuckDbError::Timeout { .. }), "{err:?}");
    wait_quiet(&db).await;
}

#[tokio::test]
async fn the_wait_for_a_connection_counts_in_the_timeout() {
    let db = db_with(|c| {
        c.timeout_ms = 300;
        c.max_connections = 1;
    })
    .await;
    let busy = db.clone();
    // An interrupt does not stop a sleep. The lease stays out for one second.
    let slow = tokio::spawn(async move {
        busy.with_connection(|_| {
            std::thread::sleep(Duration::from_secs(1));
            Ok(())
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let err = db.query("SELECT 1").fetch().await.unwrap_err();
    assert!(matches!(err, DuckDbError::Timeout { .. }), "{err:?}");
    assert!(slow.await.unwrap().is_err());
    wait_quiet(&db).await;
}

#[tokio::test]
async fn a_dropped_call_interrupts_the_query() {
    let db = db().await;
    let busy = db.clone();
    let task = tokio::spawn(async move { busy.query(SLOW).fetch().await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    wait_quiet(&db).await;
    assert_eq!(counter(&db, "duckdb_calls_total", Some("cancelled")), 1.0);
    assert_eq!(counter(&db, "duckdb_calls_open", None), 0.0);
}

#[tokio::test]
async fn shutdown_interrupts_open_calls_and_refuses_new_ones() {
    let db = db().await;
    let busy = db.clone();
    let task = tokio::spawn(async move { busy.query(SLOW).fetch().await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    db.shutdown().await;
    assert_eq!(task.await.unwrap().unwrap_err(), DuckDbError::ShuttingDown);
    assert_eq!(
        db.query("SELECT 1").fetch().await.unwrap_err(),
        DuckDbError::ShuttingDown
    );
    assert_eq!(db.ping().await.unwrap_err(), DuckDbError::ShuttingDown);
    assert!(db.inner.pool.is_quiet());
}

#[tokio::test]
async fn shutdown_checkpoints_a_writable_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.duckdb");
    let wal = dir.path().join("data.duckdb.wal");
    let db = db_with(|c| c.path = path.to_str().unwrap().to_owned()).await;
    with_items(&db).await;
    assert!(wal.exists(), "the insert must be in the WAL");
    db.shutdown().await;
    assert!(
        std::fs::metadata(&wal).map_or(true, |m| m.len() == 0),
        "the checkpoint must empty the WAL"
    );
}

#[tokio::test]
async fn shutdown_skips_the_checkpoint_when_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.duckdb");
    let wal = dir.path().join("data.duckdb.wal");
    let db = db_with(|c| {
        c.path = path.to_str().unwrap().to_owned();
        c.checkpoint_on_shutdown = false;
    })
    .await;
    with_items(&db).await;
    db.shutdown().await;
    assert!(std::fs::metadata(&wal).is_ok_and(|m| m.len() > 0));
}

#[tokio::test]
async fn with_connection_gives_the_full_api() {
    let db = db().await;
    with_items(&db).await;
    let total = db
        .with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute("INSERT INTO items VALUES (4, 'd')", [])?;
            tx.commit()?;
            conn.query_row("SELECT sum(id) FROM items", [], |row| row.get::<_, i64>(0))
        })
        .await
        .unwrap();
    assert_eq!(total, 10);
    assert_eq!(counter(&db, "duckdb_calls_total", Some("succeeded")), 3.0);
}

#[tokio::test]
async fn with_connection_maps_errors_and_panics() {
    let db = db().await;
    let err = db
        .with_connection(|conn| conn.execute_batch("SELECT * FROM nothing"))
        .await
        .unwrap_err();
    assert_eq!(err.class(), Some("Catalog"));
    let err = db
        .with_connection(|_| -> duckdb::Result<()> { panic!("boom") })
        .await
        .unwrap_err();
    assert_eq!(err, DuckDbError::TaskFailed);
    wait_quiet(&db).await;
    db.query("SELECT 1").fetch().await.unwrap();
}

#[tokio::test]
async fn ping_is_not_counted() {
    let db = db().await;
    db.ping().await.unwrap();
    assert_eq!(counter(&db, "duckdb_calls_started_total", None), 0.0);
}

#[tokio::test]
async fn rows_are_counted() {
    let db = db().await;
    db.query("SELECT * FROM range(5)").fetch().await.unwrap();
    assert_eq!(counter(&db, "duckdb_rows_returned_total", None), 5.0);
}

#[tokio::test]
async fn open_validates_the_config() {
    let mut config = DuckDbConfig::default();
    config.max_rows = 0;
    assert!(matches!(
        DuckDb::open(config).await,
        Err(DuckDbError::Config(_))
    ));
}

#[tokio::test]
async fn open_runs_setup_hooks() {
    let setup: Setup = Arc::new(|conn: &Connection| conn.execute_batch("CREATE TABLE s (x INT)"));
    let db = DuckDb::open_with(DuckDbConfig::default(), vec![setup], Arc::default())
        .await
        .unwrap();
    db.query("SELECT * FROM s").fetch().await.unwrap();
}

#[tokio::test]
async fn debug_text_hides_the_sql_and_the_values() {
    let db = db().await;
    let query = db.query("SELECT secret_column").bind("secret value");
    let text = format!("{query:?} {db:?}");
    assert!(!text.contains("secret"), "{text}");
}

#[test]
fn outcomes_follow_the_result() {
    assert_eq!(outcome(&Ok(())), Outcome::Succeeded);
    assert_eq!(
        outcome::<()>(&Err(DuckDbError::Timeout {
            timeout: Duration::ZERO
        })),
        Outcome::TimedOut
    );
    assert_eq!(
        outcome::<()>(&Err(DuckDbError::ShuttingDown)),
        Outcome::Cancelled
    );
    assert_eq!(
        outcome::<()>(&Err(DuckDbError::Cancelled)),
        Outcome::Cancelled
    );
    assert_eq!(outcome::<()>(&Err(DuckDbError::NotFound)), Outcome::Failed);
}

#[tokio::test]
async fn ping_works_when_each_connection_is_busy() {
    let db = db_with(|c| c.max_connections = 1).await;
    let busy = db.clone();
    let slow = tokio::spawn(async move {
        busy.with_connection(|_| {
            std::thread::sleep(Duration::from_millis(500));
            Ok(())
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let start = Instant::now();
    db.ping().await.unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(400),
        "the ping waited for the pool"
    );
    slow.await.unwrap().unwrap();
}

#[test]
fn a_timed_out_query_stops_when_the_runtime_stops() {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let db = db_with(|c| c.timeout_ms = 50).await;
            assert!(db.query(SLOW).fetch().await.is_err());
        });
        // The drop waits for the blocking tasks.
        drop(runtime);
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the runtime did not stop: the query still runs");
}
