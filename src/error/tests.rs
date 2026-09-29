use super::*;

fn database(message: &str) -> DuckDbError {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY); INSERT INTO t VALUES (1)")
        .unwrap();
    let err = conn.execute_batch(message).unwrap_err();
    DuckDbError::from(err)
}

#[test]
fn the_class_is_the_text_before_error() {
    assert_eq!(
        class_of("Catalog Error: Table with name x does not exist!"),
        "Catalog"
    );
    assert_eq!(class_of("Invalid Input Error: bad"), "Invalid Input");
    assert_eq!(class_of("INTERRUPT Error: Interrupted!"), "INTERRUPT");
    assert_eq!(
        class_of("TransactionContext Error: Conflict"),
        "TransactionContext"
    );
}

#[test]
fn a_message_without_a_class_gives_unknown() {
    assert_eq!(class_of("something broke"), "Unknown");
    assert_eq!(class_of(""), "Unknown");
    assert_eq!(class_of("Error: no class"), "Unknown");
    assert_eq!(class_of("Two Lines\nError: x"), "Unknown");
    assert_eq!(class_of("Bad: Error: x"), "Unknown");
}

#[test]
fn a_duckdb_failure_keeps_the_class_and_the_detail() {
    let err = database("SELECT * FROM missing_table");
    assert_eq!(err.class(), Some("Catalog"));
    assert!(err.detail().unwrap().contains("missing_table"));
    assert_eq!(err.to_string(), "DuckDB refused the call: Catalog error");
}

#[test]
fn the_error_text_hides_the_detail() {
    let err = database("INSERT INTO t VALUES (1)");
    assert_eq!(err.class(), Some("Constraint"));
    assert!(err.detail().unwrap().contains("id: 1"));
    assert!(!err.to_string().contains("id: 1"));
}

#[test]
fn a_client_error_gives_the_client_class() {
    let err = DuckDbError::from(duckdb::Error::QueryReturnedNoRows);
    assert_eq!(err.class(), Some("Client"));
    assert!(err.detail().is_some());
}

#[test]
fn other_errors_have_no_class_or_detail() {
    assert_eq!(DuckDbError::ShuttingDown.class(), None);
    assert_eq!(DuckDbError::ShuttingDown.detail(), None);
}

fn with_class(class: &str, detail: &str) -> DuckDbError {
    DuckDbError::Database {
        class: class.into(),
        detail: detail.into(),
    }
}

#[test]
fn errors_map_to_http_status() {
    let timeout = DuckDbError::Timeout {
        timeout: Duration::from_secs(1),
    };
    assert_eq!(timeout.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        with_class("Constraint", "dup").status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        with_class("TransactionContext", "Catalog write-write conflict").status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        with_class("TransactionContext", "no transaction is active").status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(DuckDbError::NotFound.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        DuckDbError::Cancelled.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        DuckDbError::ShuttingDown.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        with_class("Catalog", "x").status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        DuckDbError::NotInstalled.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn timeouts_and_write_conflicts_are_retryable() {
    let timeout = DuckDbError::Timeout {
        timeout: Duration::from_secs(1),
    };
    assert!(timeout.is_retryable());
    assert!(with_class("TransactionContext", "Conflict on tuple deletion").is_retryable());
    assert!(!with_class("Constraint", "dup").is_retryable());
    assert!(!DuckDbError::ShuttingDown.is_retryable());
}

#[test]
fn into_autumn_keeps_the_status() {
    let err = with_class("Constraint", "dup").into_autumn();
    assert_eq!(err.status(), StatusCode::CONFLICT);
}

#[test]
fn or_http_converts_the_error() {
    let result: Result<(), DuckDbError> = Err(DuckDbError::ShuttingDown);
    assert_eq!(
        result.or_http().unwrap_err().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let ok: Result<u8, DuckDbError> = Ok(1);
    assert_eq!(ok.or_http().unwrap(), 1);
}
