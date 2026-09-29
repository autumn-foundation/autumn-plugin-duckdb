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
use crate::health::DatabaseCheck;
use crate::metrics::Metrics;
use crate::pool::Setup;

/// The plugin name in Autumn diagnostics.
pub const PLUGIN_NAME: &str = "autumn-plugin-duckdb";

/// The interval of the shutdown watch.
const SHUTDOWN_WATCH: std::time::Duration = std::time::Duration::from_millis(200);

/// State that the plugin hooks share.
#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) handle: OnceLock<DuckDb>,
    pub(crate) metrics: Arc<Metrics>,
}

impl Shared {
    pub(crate) async fn shutdown(&self) {
        if let Some(db) = self.handle.get() {
            db.shutdown().await;
        }
    }
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
        let mut config = match source {
            Source::Section(section) => DuckDbConfig::resolve(section)?,
            Source::Explicit(config) => (**config).clone(),
        };
        for change in changes {
            change(&mut config);
        }
        match source {
            Source::Section(section) => config.validate_section(section)?,
            Source::Explicit(_) => config.validate()?,
        }
        Ok(config)
    }
}

impl Plugin for DuckDbPlugin {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(PLUGIN_NAME)
    }

    fn build(self, app: AppBuilder) -> AppBuilder {
        let Self {
            source,
            changes,
            setups,
        } = self;
        let mut app = app;
        if let Source::Section(section) = &source {
            app = app.config_section(section.clone());
        }
        let resolved = Self::resolve(&source, changes);
        let shared = Arc::new(Shared::default());
        app = app.metrics_source("duckdb", Arc::clone(&shared.metrics) as _);
        if resolved.as_ref().is_ok_and(|config| config.health_check) {
            let check = DatabaseCheck::new(Arc::clone(&shared));
            app = app.health_indicator("duckdb", Arc::new(check));
        }
        let on_start = Arc::clone(&shared);
        let resolved = Arc::new(resolved);
        app.on_startup(move |state| {
            let shared = Arc::clone(&on_start);
            let resolved = Arc::clone(&resolved);
            let setups = setups.clone();
            async move {
                let config = resolved.as_ref().clone().map_err(|err| {
                    AutumnError::internal_server_error_msg(format!("{PLUGIN_NAME}: {err}"))
                })?;
                let db = DuckDb::open_with(config, setups, Arc::clone(&shared.metrics))
                    .await
                    .map_err(|err| {
                        // The DuckDB message can hold SQL text. The boot error shows the class only.
                        AutumnError::internal_server_error_msg(format!("{PLUGIN_NAME}: {err}"))
                    })?;
                state.insert_extension(db.clone());
                let _ = shared.handle.set(db.clone());
                tokio::spawn(watch_shutdown(state, db));
                tracing::info!("the DuckDB plugin is ready");
                Ok(())
            }
        })
        .on_shutdown(move || {
            let shared = Arc::clone(&shared);
            async move { shared.shutdown().await }
        })
    }
}

/// Shuts the handle down when Autumn marks the shutdown.
///
/// Autumn runs the shutdown hooks after the request drain. The drain can end the process first.
async fn watch_shutdown(state: AppState, db: DuckDb) {
    while !state.probes().is_shutting_down() {
        tokio::time::sleep(SHUTDOWN_WATCH).await;
    }
    tracing::info!("the app shuts down: interrupting the open DuckDB calls");
    db.shutdown().await;
}

impl std::fmt::Debug for DuckDbPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let source = match &self.source {
            Source::Section(section) => section.as_str(),
            Source::Explicit(_) => "(explicit)",
        };
        f.debug_struct("DuckDbPlugin")
            .field("config", &source)
            .field("changes", &self.changes.len())
            .field("setups", &self.setups.len())
            .finish()
    }
}

impl DuckDb {
    /// Gets the handle from the app state, for example in a job or a task.
    #[must_use]
    pub fn from_state(state: &AppState) -> Option<Self> {
        state.extension::<Self>().map(|db| (*db).clone())
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
