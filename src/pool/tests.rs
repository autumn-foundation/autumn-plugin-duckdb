#![allow(
    clippy::field_reassign_with_default,
    reason = "each test changes one key of the defaults"
)]

use std::time::Duration;

use super::*;

fn open_default() -> Connection {
    open(&DuckDbConfig::default(), &[]).unwrap()
}

fn setting(conn: &Connection, name: &str) -> String {
    conn.query_row(&format!("SELECT current_setting('{name}')::VARCHAR"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn class_of(result: duckdb::Result<()>) -> String {
    DuckDbError::from(result.unwrap_err())
        .class()
        .unwrap()
        .to_owned()
}

/// Writes a CSV file and gives its path as SQL-safe text.
fn csv(dir: &std::path::Path, name: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, "x\n1\n").unwrap();
    path.to_str().unwrap().replace('\'', "''")
}

fn read_csv(conn: &Connection, path: &str) -> duckdb::Result<()> {
    conn.execute_batch(&format!("SELECT * FROM read_csv('{path}')"))
}

#[test]
fn external_access_is_off_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let file = csv(dir.path(), "a.csv");
    let root = open_default();
    assert_eq!(class_of(read_csv(&root, &file)), "Permission");
    let other = root.try_clone().unwrap();
    assert_eq!(class_of(read_csv(&other, &file)), "Permission");
}

#[test]
fn allowed_directories_can_be_read() {
    let allowed = tempfile::tempdir().unwrap();
    let denied = tempfile::tempdir().unwrap();
    let mut config = DuckDbConfig::default();
    config.allowed_directories = vec![format!("{}/", allowed.path().display())];
    let conn = open(&config, &[]).unwrap();
    read_csv(&conn, &csv(allowed.path(), "ok.csv")).unwrap();
    assert_eq!(
        class_of(read_csv(&conn, &csv(denied.path(), "no.csv"))),
        "Permission"
    );
}

#[test]
fn external_access_can_be_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = DuckDbConfig::default();
    config.enable_external_access = true;
    let conn = open(&config, &[]).unwrap();
    read_csv(&conn, &csv(dir.path(), "a.csv")).unwrap();
}

#[test]
fn the_configuration_is_locked_by_default() {
    let conn = open_default().try_clone().unwrap();
    assert_eq!(
        class_of(conn.execute_batch("SET threads = 1")),
        "Invalid Input"
    );
    assert_eq!(
        class_of(conn.execute_batch("SET lock_configuration = false")),
        "Invalid Input"
    );
}

#[test]
fn the_lock_can_be_off() {
    let mut config = DuckDbConfig::default();
    config.lock_configuration = false;
    let conn = open(&config, &[]).unwrap();
    conn.execute_batch("SET threads = 1").unwrap();
}

#[test]
fn extension_flags_are_off_by_default() {
    let conn = open_default();
    assert_eq!(setting(&conn, "autoinstall_known_extensions"), "false");
    assert_eq!(setting(&conn, "autoload_known_extensions"), "false");
}

#[test]
fn config_values_reach_duckdb() {
    let mut config = DuckDbConfig::default();
    config.threads = Some(2);
    config.memory_limit = Some("512MB".into());
    config.autoinstall_extensions = true;
    config.autoload_extensions = true;
    config
        .settings
        .insert("default_order".into(), "desc".into());
    let conn = open(&config, &[]).unwrap();
    assert_eq!(setting(&conn, "threads"), "2");
    assert_eq!(setting(&conn, "memory_limit"), "488.2 MiB");
    assert_eq!(setting(&conn, "autoinstall_known_extensions"), "true");
    assert_eq!(setting(&conn, "autoload_known_extensions"), "true");
    assert_eq!(setting(&conn, "default_order"), "desc");
}

#[test]
fn a_bad_memory_limit_fails_the_open() {
    let mut config = DuckDbConfig::default();
    config.memory_limit = Some("lots".into());
    assert!(open(&config, &[]).is_err());
}

#[test]
fn setup_hooks_run_in_order_with_file_access() {
    let dir = tempfile::tempdir().unwrap();
    let file = csv(dir.path(), "seed.csv");
    let load: Setup = Arc::new(move |conn: &Connection| {
        conn.execute_batch(&format!(
            "CREATE TABLE seed AS SELECT * FROM read_csv('{file}')"
        ))
    });
    let count: Setup = Arc::new(|conn: &Connection| {
        conn.execute_batch("CREATE TABLE seen AS SELECT count(*) AS n FROM seed")
    });
    let conn = open(&DuckDbConfig::default(), &[load, count]).unwrap();
    let n: i64 = conn
        .query_row("SELECT n FROM seen", [], |row| row.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn a_failed_setup_hook_fails_the_open() {
    let bad: Setup = Arc::new(|conn: &Connection| conn.execute_batch("SELECT * FROM nothing"));
    let err = open(&DuckDbConfig::default(), &[bad]).unwrap_err();
    assert_eq!(err.class(), Some("Catalog"));
}

#[test]
fn a_read_only_file_refuses_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ro.duckdb");
    let mut config = DuckDbConfig::default();
    config.path = path.to_str().unwrap().to_owned();
    drop(open(&config, &[Arc::new(|c: &Connection| c.execute_batch("CREATE TABLE t (x INT)"))]).unwrap());
    config.access_mode = crate::config::AccessMode::ReadOnly;
    let conn = open(&config, &[]).unwrap();
    conn.execute_batch("SELECT * FROM t").unwrap();
    assert!(conn.execute_batch("INSERT INTO t VALUES (1)").is_err());
}

fn pool(size: usize) -> Arc<Pool> {
    let mut config = DuckDbConfig::default();
    config.lock_configuration = false;
    Pool::new(open(&config, &[]).unwrap(), size)
}

#[tokio::test]
async fn a_lease_gives_a_connection_and_returns_it() {
    let pool = pool(2);
    assert!(pool.is_quiet());
    let lease = pool.acquire().await.unwrap();
    assert!(!pool.is_quiet());
    lease.connection().execute_batch("SELECT 1").unwrap();
    lease.release();
    assert!(pool.is_quiet());
    assert_eq!(pool.idle(), 1);
}

#[tokio::test]
async fn the_permit_limits_the_leases() {
    let pool = pool(1);
    let first = pool.acquire().await.unwrap();
    let waiting = tokio::time::timeout(Duration::from_millis(50), pool.acquire()).await;
    assert!(waiting.is_err(), "a second lease must wait");
    first.release();
    let second = tokio::time::timeout(Duration::from_secs(5), pool.acquire()).await;
    assert!(second.unwrap().is_ok());
}

#[tokio::test]
async fn a_released_connection_is_used_again() {
    let pool = pool(1);
    let lease = pool.acquire().await.unwrap();
    lease
        .connection()
        .execute_batch("CREATE TEMP TABLE mine (x INT)")
        .unwrap();
    lease.release();
    let lease = pool.acquire().await.unwrap();
    lease.connection().execute_batch("SELECT * FROM mine").unwrap();
}

#[tokio::test]
async fn release_rolls_back_an_open_transaction() {
    let pool = pool(1);
    let lease = pool.acquire().await.unwrap();
    lease
        .connection()
        .execute_batch("CREATE TABLE t (x INT); BEGIN; INSERT INTO t VALUES (1);")
        .unwrap();
    lease.release();
    assert_eq!(pool.idle(), 1);
    let lease = pool.acquire().await.unwrap();
    let n: i64 = lease
        .connection()
        .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(n, 0);
    lease.connection().execute_batch("BEGIN; COMMIT;").unwrap();
}

#[tokio::test]
async fn a_dropped_lease_drops_its_connection_and_frees_the_permit() {
    let pool = pool(1);
    drop(pool.acquire().await.unwrap());
    assert!(pool.is_quiet());
    assert_eq!(pool.idle(), 0);
    assert!(pool.acquire().await.is_ok());
}

#[tokio::test]
async fn close_refuses_leases_and_drops_idle_connections() {
    let pool = pool(2);
    let kept = pool.acquire().await.unwrap();
    pool.acquire().await.unwrap().release();
    assert_eq!(pool.idle(), 1);
    pool.close();
    assert_eq!(pool.idle(), 0);
    assert!(matches!(
        pool.acquire().await,
        Err(DuckDbError::ShuttingDown)
    ));
    kept.release();
    assert_eq!(pool.idle(), 0);
    assert!(pool.is_quiet());
}

#[test]
fn checkpoint_runs_on_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = DuckDbConfig::default();
    config.path = dir.path().join("c.duckdb").to_str().unwrap().to_owned();
    let pool = Pool::new(open(&config, &[]).unwrap(), 1);
    pool.checkpoint().unwrap();
}
