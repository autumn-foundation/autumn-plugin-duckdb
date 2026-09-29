//! A small app that reads sales totals from DuckDB.
//!
//! The setup hook makes and fills a table in an in-memory database. Run the command below.
//! Then open `http://localhost:3000/totals/2024`.
//!
//! ```sh
//! cargo run --example sales
//! ```

use autumn_plugin_duckdb::{DuckDb, DuckDbPlugin, DuckDbResultExt as _};
use autumn_web::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Total {
    region: String,
    total: f64,
}

#[get("/totals/{year}")]
async fn totals(db: DuckDb, Path(year): Path<i32>) -> AutumnResult<Json<Vec<Total>>> {
    let rows = db
        .query(
            "SELECT region, sum(amount) AS total FROM sales
             WHERE year = ? GROUP BY region ORDER BY region",
        )
        .bind(year)
        .fetch_as::<Total>()
        .await
        .or_http()?;
    Ok(Json(rows))
}

#[autumn_web::main]
async fn main() {
    autumn_web::app()
        .plugin(DuckDbPlugin::new().setup(|conn| {
            conn.execute_batch(
                "CREATE TABLE sales (region VARCHAR, year INTEGER, amount DECIMAL(12, 2));
                 INSERT INTO sales VALUES
                     ('north', 2024, 120.50), ('south', 2024, 80.00), ('north', 2023, 99.99);",
            )
        }))
        .routes(routes![totals])
        .run()
        .await;
}
