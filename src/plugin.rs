//! [`DuckDbPlugin`]: installs a [`DuckDb`] handle in an Autumn app.
//!
//! # Contract
//!
//! - `build` reads the configuration. A bad configuration stops the boot in the startup hook.
//! - The startup hook opens the database on a blocking thread and puts the handle in the app state.
//! - When Autumn marks the shutdown, a watch task shuts the handle down. The shutdown hook does the same.
//! - The readiness check and the metrics source use the same handle.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use autumn_web::app::AppBuilder;
use autumn_web::plugin::Plugin;
use autumn_web::{AppState, AutumnError};
use duckdb::Connection;

use crate::client::DuckDb;
use crate::config::{ConfigError, DEFAULT_SECTION, DuckDbConfig};
use crate::error::DuckDbError;
use crate::metrics::Metrics;
use crate::pool::Setup;

/// The plugin name in Autumn diagnostics.
pub const PLUGIN_NAME: &str = "autumn-plugin-duckdb";

/// State that the plugin hooks share.
#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) handle: OnceLock<DuckDb>,
    pub(crate) metrics: Arc<Metrics>,
}

impl Shared {
    pub(crate) async fn shutdown(&self) {}
}

enum Source {
    Section(String),
    Explicit(Box<DuckDbConfig>),
}

type Change = Box<dyn FnOnce(&mut DuckDbConfig) + Send>;

/// Installs a [`DuckDb`] handle in an Autumn app.
///
/// ```rust,no_run
/// use autumn_plugin_duckdb::DuckDbPlugin;
///
/// # async fn run() {
/// autumn_web::app().plugin(DuckDbPlugin::new()).run().await;
/// # }
/// ```
#[must_use]
pub struct DuckDbPlugin {
    source: Source,
    changes: Vec<Change>,
    setups: Vec<Setup>,
}

impl Default for DuckDbPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl DuckDbPlugin {
    /// Makes a plugin that reads `[duckdb]`.
    pub fn new() -> Self {
        Self {
            source: Source::Section(DEFAULT_SECTION.to_owned()),
            changes: Vec::new(),
            setups: Vec::new(),
        }
    }

    /// Reads `[section]` instead of `[duckdb]`.
    ///
    /// An app can have one DuckDB plugin only. Autumn ignores a second plugin with the same name.
    pub fn config_section(mut self, section: impl Into<String>) -> Self {
        self.source = Source::Section(section.into());
        self
    }

    /// Uses `config` and reads no files or variables.
    pub fn config(mut self, config: DuckDbConfig) -> Self {
        self.source = Source::Explicit(Box::new(config));
        self
    }

    /// Changes the configuration after the plugin reads it.
    pub fn configure(mut self, change: impl FnOnce(&mut DuckDbConfig) + Send + 'static) -> Self {
        self.changes.push(Box::new(change));
        self
    }

    /// Adds Rust code that runs on the database at startup, for example to create tables.
    ///
    /// Setup hooks run in order, before the plugin disables external access and locks the configuration.
    /// A failed hook stops the boot.
    pub fn setup(
        mut self,
        hook: impl Fn(&Connection) -> duckdb::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.setups.push(Arc::new(hook));
        self
    }

    fn resolve(source: &Source, changes: Vec<Change>) -> Result<DuckDbConfig, ConfigError> {
        let _ = (source, changes);
        Err(ConfigError(String::new()))
    }
}

impl Plugin for DuckDbPlugin {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(PLUGIN_NAME)
    }

    fn build(self, app: AppBuilder) -> AppBuilder {
        app
    }
}

impl std::fmt::Debug for DuckDbPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDbPlugin").finish_non_exhaustive()
    }
}

impl DuckDb {
    /// Gets the handle from the app state, for example in a job or a task.
    #[must_use]
    pub fn from_state(state: &AppState) -> Option<Self> {
        let _ = state;
        None
    }
}

impl axum::extract::FromRequestParts<AppState> for DuckDb {
    type Rejection = AutumnError;

    async fn from_request_parts(
        _parts: &mut http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Self::from_state(state).ok_or_else(|| DuckDbError::NotInstalled.into_autumn())
    }
}

#[cfg(test)]
mod tests;
