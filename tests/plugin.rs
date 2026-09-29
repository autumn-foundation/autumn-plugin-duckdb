//! The plugin in an Autumn test app.

use autumn_plugin_duckdb::{DuckDb, DuckDbConfig, DuckDbError, DuckDbPlugin, DuckDbResultExt as _};
use autumn_web::prelude::*;
use autumn_web::test::{TestApp, TestClient};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Item {
    id: i64,
    name: String,
}

#[get("/items")]
async fn items(db: DuckDb) -> AutumnResult<Json<Vec<Item>>> {
    let rows = db
        .query("SELECT id, name FROM items WHERE name <> ? ORDER BY id")
        .bind("nobody")
        .fetch_as::<Item>()
        .await
        .or_http()?;
    Ok(Json(rows))
}

#[get("/items/{id}")]
async fn item(db: DuckDb, Path(id): Path<i64>) -> AutumnResult<Json<Item>> {
    let item = db
        .query("SELECT id, name FROM items WHERE id = ?")
        .bind(id)
        .fetch_one_as::<Item>()
        .await
        .or_http()?;
    Ok(Json(item))
}

#[post("/items/{id}")]
async fn add(db: DuckDb, Path(id): Path<i64>) -> AutumnResult<&'static str> {
    db.query("INSERT INTO items VALUES (?, 'new')")
        .bind(id)
        .execute()
        .await
        .map_err(DuckDbError::into_autumn)?;
    Ok("ok")
}

#[get("/slow")]
async fn slow(db: DuckDb) -> AutumnResult<&'static str> {
    db.query("SELECT count(*) FROM range(100000) a, range(10000000) b")
        .fetch()
        .await
        .or_http()?;
    Ok("done")
}

fn plugin() -> DuckDbPlugin {
    DuckDbPlugin::new()
        .config(DuckDbConfig::default())
        .setup(|conn| {
            conn.execute_batch(
                "CREATE TABLE items (id BIGINT PRIMARY KEY, name VARCHAR);
                 INSERT INTO items VALUES (1, 'ada'), (2, 'bob');",
            )
        })
}

fn app(plugin: DuckDbPlugin) -> TestClient {
    TestApp::new()
        .routes(routes![items, item, add, slow])
        .plugin(plugin)
        .build()
}

#[tokio::test]
async fn a_handler_runs_a_query_with_the_extractor() {
    let client = app(plugin());
    let response = client.get("/items").send().await;
    response.assert_ok();
    assert_eq!(
        response.json::<Vec<Item>>(),
        vec![
            Item {
                id: 1,
                name: "ada".into()
            },
            Item {
                id: 2,
                name: "bob".into()
            }
        ]
    );
}

#[tokio::test]
async fn no_row_gives_not_found() {
    let client = app(plugin());
    client.get("/items/1").send().await.assert_ok();
    client.get("/items/9").send().await.assert_status(404);
}

#[tokio::test]
async fn a_duplicate_key_gives_a_conflict() {
    let client = app(plugin());
    client.post("/items/3").send().await.assert_ok();
    client.post("/items/3").send().await.assert_status(409);
}

#[tokio::test]
async fn a_timeout_gives_a_gateway_timeout() {
    let client = app(plugin().configure(|c| c.timeout_ms = 100));
    client.get("/slow").send().await.assert_status(504);
}

#[tokio::test]
async fn configure_changes_the_config() {
    let client = app(plugin().configure(|c| c.max_rows = 1));
    client.get("/items").send().await.assert_status(500);
    let db = DuckDb::from_state(client.state()).unwrap();
    assert_eq!(db.config().max_rows, 1);
}

#[tokio::test]
async fn the_extractor_fails_without_the_plugin() {
    let client = TestApp::new().routes(routes![items]).build();
    client.get("/items").send().await.assert_status(500);
}

#[tokio::test]
#[should_panic(expected = "Plugin startup hook failed")]
async fn a_bad_config_stops_the_boot() {
    let _ = app(DuckDbPlugin::new().configure(|c| c.max_connections = 0));
}

#[tokio::test]
#[should_panic(expected = "Plugin startup hook failed")]
async fn a_failed_setup_hook_stops_the_boot() {
    let _ = app(DuckDbPlugin::new()
        .config(DuckDbConfig::default())
        .setup(|conn| conn.execute_batch("SELECT * FROM nothing")));
}

#[tokio::test]
async fn calls_work_during_the_request_drain() {
    // Autumn marks the shutdown first. Then it drains the requests. Then it runs the shutdown hooks.
    let client = app(plugin());
    client.state().begin_shutdown_for_test();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    client.get("/items").send().await.assert_ok();
}

#[tokio::test]
async fn a_standalone_handle_can_shut_down() {
    let db = DuckDb::open(DuckDbConfig::default()).await.unwrap();
    let row = db.query("SELECT 1 AS one").fetch_one().await.unwrap();
    assert_eq!(row.columns(), ["one"]);
    db.shutdown().await;
    assert_eq!(
        db.query("SELECT 1").fetch_one().await.unwrap_err(),
        DuckDbError::ShuttingDown
    );
}
