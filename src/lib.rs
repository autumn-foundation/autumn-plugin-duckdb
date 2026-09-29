//! Autumn plugin for DuckDB.
//!
//! Add [`DuckDbPlugin`] to the app. Then use the [`DuckDb`] extractor in a handler.
//!
//! ```rust,no_run
//! use autumn_plugin_duckdb::{DuckDb, DuckDbPlugin, DuckDbResultExt as _};
//! use autumn_web::prelude::*;
//!
//! #[derive(serde::Deserialize, serde::Serialize)]
//! struct Total {
//!     region: String,
//!     total: f64,
//! }
//!
//! #[get("/totals/{year}")]
//! async fn totals(db: DuckDb, Path(year): Path<i32>) -> AutumnResult<Json<Vec<Total>>> {
//!     let rows = db
//!         .query("SELECT region, sum(amount) AS total FROM sales WHERE year = ? GROUP BY region")
//!         .bind(year)
//!         .fetch_as::<Total>()
//!         .await
//!         .or_http()?;
//!     Ok(Json(rows))
//! }
//!
//! # async fn run() {
//! autumn_web::app()
//!     .plugin(DuckDbPlugin::new().setup(|conn| {
//!         conn.execute_batch("CREATE TABLE IF NOT EXISTS sales (region TEXT, year INT, amount DOUBLE)")
//!     }))
//!     .routes(routes![totals])
//!     .run()
//!     .await;
//! # }
//! ```
//!
//! The plugin reads `[duckdb]` in `autumn.toml`. See [`config`] for the keys.
//!
//! # Security rules
//!
//! - Bind each value with [`DuckDbQuery::bind`]. Do not put values into the SQL text.
//! - A query has one statement only. The plugin refuses more.
//! - SQL cannot read or write files by default. List the directories in `allowed_directories`.
//! - DuckDB does not download or load extensions by default. Load them in a setup hook.
//! - SQL cannot change the DuckDB configuration after startup.
//! - Each call has a timeout, a row limit and a byte limit. The plugin interrupts a timed-out query.
//! - Logs do not have SQL text, parameter values or DuckDB messages.

mod client;
pub mod config;
mod decode;
mod error;
mod health;
mod metrics;
mod param;
mod plugin;
mod pool;
mod statement;
mod temporal;
mod value;

pub use client::{DuckDb, DuckDbQuery};
pub use config::{AccessMode, ConfigError, DuckDbConfig};
pub use decode::DecodeError;
/// The `duckdb` crate that the plugin uses. Use it in setup hooks and in `with_connection`.
pub use duckdb;
pub use error::{DuckDbError, DuckDbResultExt};
pub use param::Param;
pub use plugin::{DuckDbPlugin, PLUGIN_NAME};
pub use value::{Row, Value};
