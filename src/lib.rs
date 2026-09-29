//! Autumn plugin for DuckDB.

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
pub use pool::Setup;
pub use value::{Row, Value};
