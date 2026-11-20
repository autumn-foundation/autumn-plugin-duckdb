//! DuckDB fleets: one DuckDB file for each tenant or slot (feature `fleet`).
//!
//! # Contract
//!
//! [`DuckDbEngine`] is the DuckDB engine of an Autumn [`DatabaseFleet`]. Autumn core does the keys,
//! the paths, the opens, the eviction, the memory budget, the ownership and the fence.
//! The engine does the rest:
//!
//! - `open` opens the file with the engine configuration and runs the setup hooks.
//!   The table `autumn_fleet.identity` holds the key name and the epoch.
//!   One transaction checks and writes them. DuckDB lets one process open a file for writes,
//!   so the check and the stamp are atomic.
//! - `open` refuses a file of another key ([`FleetError::Misplaced`]) and a newer epoch
//!   ([`FleetError::NewerEpoch`]). It never lowers the stamp.
//! - `close` refuses new calls and interrupts open calls. With no open call, it runs `CHECKPOINT`.
//!   DuckDB closes the file when the last call ends.
//! - `snapshot` waits for all connections of the pool, runs `CHECKPOINT`, and copies the file.
//!   A file with no write since the last snapshot gives the same bytes.
//! - `remove` deletes the file and its `.wal` file.
//! - The ship mode is [`ShipMode::Generations`].
//! - [`DuckDbFleetPlugin`] reads `[duckdb_fleet]`, makes the fleet and puts it in the app state.
//!   [`FleetDuckDb`] is the extractor. It opens the database of the request tenant.
//! - With `[duckdb_fleet.ownership]`, the plugin claims its slot ranges before the first open.
//! - The plugin closes idle databases, and closes all databases at shutdown. This runs in each role:
//!   a role that opens a database must also close it.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use autumn_web::app::AppBuilder;
use autumn_web::config::{DatabaseFleetConfig, Env};
use autumn_web::db::fleet::{DatabaseFleet, FleetError};
use autumn_web::db::fleet_engine::{FleetEngine, ShipMode};
use autumn_web::db::fleet_ownership::{FleetOwnership, OWNERSHIP_INDICATOR, OwnerId, SlotRange};
use autumn_web::fleet::FleetLease;
use autumn_web::fleet_layout::FleetDbKey;
use autumn_web::lock::FencingToken;
use autumn_web::plugin::Plugin;
use autumn_web::reexports::tokio_util::sync::CancellationToken;
use autumn_web::{AppState, AutumnError};
use duckdb::Connection;

use crate::client::DuckDb;
use crate::config::{AccessMode, ConfigError, DuckDbConfig, layered_section};
use crate::error::DuckDbError;
use crate::metrics::Metrics;
use crate::pool::Setup;

/// The plugin name of [`DuckDbFleetPlugin`] in Autumn diagnostics.
pub const FLEET_PLUGIN_NAME: &str = "autumn-plugin-duckdb-fleet";

/// The default section of [`DuckDbFleetConfig`].
pub const DEFAULT_FLEET_SECTION: &str = "duckdb_fleet";

/// The default interval between two shipped generations.
pub const DEFAULT_SHIP_EVERY: Duration = Duration::from_mins(1);

/// The default memory that the fleet counts for one open database: 256 MiB.
pub const DEFAULT_RESIDENT_BYTES: u64 = 256 * 1024 * 1024;

/// The DuckDB engine of an Autumn fleet. See the module contract.
pub struct DuckDbEngine {
    template: DuckDbConfig,
    setups: Vec<Setup>,
    metrics: Arc<Metrics>,
    ship_every: Duration,
    resident_bytes: u64,
}

impl DuckDbEngine {
    /// Makes an engine that opens each file with `template`. The fleet sets the path.
    #[must_use]
    pub fn new(template: DuckDbConfig) -> Self {
        Self {
            template,
            setups: Vec::new(),
            metrics: Arc::default(),
            ship_every: DEFAULT_SHIP_EVERY,
            resident_bytes: DEFAULT_RESIDENT_BYTES,
        }
    }

    /// Sets the interval between two shipped generations.
    #[must_use]
    pub const fn ship_every(mut self, every: Duration) -> Self {
        self.ship_every = every;
        self
    }

    /// Sets the memory that the fleet counts for one open database.
    ///
    /// Set it near the DuckDB `memory_limit`. The fleet budget closes idle databases with it.
    #[must_use]
    pub const fn resident_bytes(mut self, bytes: u64) -> Self {
        self.resident_bytes = bytes;
        self
    }

    /// Adds Rust code that runs on each database at open, for example to create tables.
    #[must_use]
    pub fn setup(
        mut self,
        hook: impl Fn(&Connection) -> duckdb::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.setups.push(Arc::new(hook));
        self
    }

    fn config_for(&self, path: &Path) -> DuckDbConfig {
        let mut config = self.template.clone();
        config.path = path.display().to_string();
        config
    }
}

impl std::fmt::Debug for DuckDbEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDbEngine")
            .field("setups", &self.setups.len())
            .field("ship_every", &self.ship_every)
            .field("resident_bytes", &self.resident_bytes)
            .finish_non_exhaustive()
    }
}

/// Makes an I/O fleet error. The text has the error class only, not the DuckDB message.
fn engine_error(op: &'static str) -> impl FnOnce(DuckDbError) -> FleetError {
    move |err| FleetError::Io {
        op,
        detail: err.class().map_or_else(|| err.to_string(), str::to_owned),
    }
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> FleetError {
    move |err| FleetError::Io {
        op,
        detail: err.to_string(),
    }
}

/// The DuckDB `.wal` file of `path`.
fn wal_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".wal");
    PathBuf::from(name)
}

/// Checks and writes the key and the epoch in one transaction.
fn stamp(conn: &Connection, name: &str, epoch: u64) -> duckdb::Result<Result<(), FleetError>> {
    conn.execute_batch(
        "BEGIN TRANSACTION;
         CREATE SCHEMA IF NOT EXISTS autumn_fleet;
         CREATE TABLE IF NOT EXISTS autumn_fleet.identity (name VARCHAR NOT NULL, epoch UBIGINT NOT NULL);",
    )?;
    let decided = decide(conn, name, epoch);
    match decided {
        Ok(Ok(())) => conn.execute_batch("COMMIT")?,
        _ => conn.execute_batch("ROLLBACK")?,
    }
    decided
}

fn decide(conn: &Connection, name: &str, epoch: u64) -> duckdb::Result<Result<(), FleetError>> {
    let found: Option<(String, u64)> = {
        let mut statement = conn.prepare("SELECT name, epoch FROM autumn_fleet.identity")?;
        let mut rows = statement.query([])?;
        match rows.next()? {
            Some(row) => Some((row.get(0)?, row.get(1)?)),
            None => None,
        }
    };
    let Some((found, stamp)) = found else {
        conn.execute(
            "INSERT INTO autumn_fleet.identity VALUES (?, ?)",
            duckdb::params![name, epoch],
        )?;
        return Ok(Ok(()));
    };
    if found != name {
        return Ok(Err(FleetError::Misplaced {
            name: name.to_owned(),
            found,
        }));
    }
    if epoch > 0 && stamp > epoch {
        return Ok(Err(FleetError::NewerEpoch {
            name: name.to_owned(),
            epoch,
            found: stamp,
        }));
    }
    if epoch > stamp {
        conn.execute(
            "UPDATE autumn_fleet.identity SET epoch = ?",
            duckdb::params![epoch],
        )?;
    }
    Ok(Ok(()))
}

impl FleetEngine for DuckDbEngine {
    type Handle = DuckDb;

    async fn open(
        &self,
        key: &FleetDbKey,
        path: &Path,
        epoch: Option<FencingToken>,
    ) -> Result<DuckDb, FleetError> {
        if let Some(parent) = path.parent() {
            let parent = parent.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::create_dir_all(parent))
                .await
                .map_err(|_| FleetError::Io {
                    op: "create directory",
                    detail: "the blocking task stopped".to_owned(),
                })?
                .map_err(io_error("create directory"))?;
        }
        let db = DuckDb::open_with(
            self.config_for(path),
            self.setups.clone(),
            Arc::clone(&self.metrics),
        )
        .await
        .map_err(engine_error("open database"))?;
        let name = key.name();
        let epoch = epoch.map_or(0, FencingToken::get);
        let stamped = db
            .exclusive(move |conn| Ok(stamp(conn, &name, epoch)?))
            .await
            .map_err(engine_error("stamp database"))
            .and_then(|stamped| stamped);
        if let Err(err) = stamped {
            db.close_now();
            return Err(err);
        }
        Ok(db)
    }

    fn close(&self, handle: &DuckDb) {
        handle.close_now();
    }

    fn in_use(&self, handle: &DuckDb) -> bool {
        handle.is_busy()
    }

    async fn snapshot(
        &self,
        handle: &DuckDb,
        path: &Path,
        dir: &Path,
    ) -> Result<Vec<PathBuf>, FleetError> {
        let file = path.file_name().ok_or_else(|| FleetError::Io {
            op: "snapshot database",
            detail: "the path has no file name".to_owned(),
        })?;
        let copy = dir.join(file);
        let source = path.to_path_buf();
        let target = copy.clone();
        if handle.is_closed() {
            tokio::task::spawn_blocking(move || std::fs::copy(source, target))
                .await
                .map_err(|_| FleetError::Io {
                    op: "snapshot database",
                    detail: "the blocking task stopped".to_owned(),
                })?
                .map_err(io_error("snapshot database"))?;
        } else {
            handle
                .exclusive(move |root| {
                    root.execute_batch("CHECKPOINT")?;
                    Ok(std::fs::copy(source, target))
                })
                .await
                .map_err(engine_error("snapshot database"))?
                .map_err(io_error("snapshot database"))?;
        }
        Ok(vec![copy])
    }

    fn remove(&self, _key: &FleetDbKey, path: &Path) -> Result<bool, FleetError> {
        let removed = match std::fs::remove_file(path) {
            Ok(()) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => return Err(io_error("delete database")(err)),
        };
        match std::fs::remove_file(wal_of(path)) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_error("delete database")(err)),
        }
        Ok(removed)
    }

    fn resident_bytes(&self, _handle: &DuckDb) -> u64 {
        self.resident_bytes
    }

    fn ship_mode(&self) -> ShipMode {
        ShipMode::Generations {
            every: self.ship_every,
        }
    }

    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn sidecars(&self, path: &Path) -> Vec<PathBuf> {
        vec![wal_of(path)]
    }

    fn epoch_stamp(&self, _key: &FleetDbKey, path: &Path) -> Result<Option<u64>, FleetError> {
        if !path.is_file() {
            return Ok(None);
        }
        let read = || -> duckdb::Result<Option<u64>> {
            let flags = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?;
            let conn = Connection::open_with_flags(path, flags)?;
            let known: i64 = conn.query_row(
                "SELECT count(*) FROM duckdb_tables() \
                 WHERE schema_name = 'autumn_fleet' AND table_name = 'identity'",
                [],
                |row| row.get(0),
            )?;
            if known == 0 {
                return Ok(None);
            }
            let mut statement = conn.prepare("SELECT epoch FROM autumn_fleet.identity")?;
            let mut rows = statement.query([])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        };
        read().map_err(|err| engine_error("read epoch stamp")(DuckDbError::from(err)))
    }
}

/// The DuckDB database of the request tenant, from a [`DuckDbFleetPlugin`] fleet.
///
/// It derefs to [`DuckDb`]. The fleet keeps the database open while the value lives.
///
/// The rejection is the fleet error status: `400` with no tenant, `421` when another host owns the
/// slot, and `503` with no fleet.
#[derive(Debug, Clone)]
pub struct FleetDuckDb(pub FleetLease<DuckDbEngine>);

impl FleetDuckDb {
    /// Opens the database of `key` with no request, for example in a job or a task.
    ///
    /// # Errors
    ///
    /// Returns the [`FleetError`] of [`autumn_web::fleet::resolve_key`].
    pub async fn for_key(state: &AppState, key: &str) -> Result<Self, FleetError> {
        autumn_web::fleet::resolve_key::<DuckDbEngine>(state, key)
            .await
            .map(Self)
    }
}

impl std::ops::Deref for FleetDuckDb {
    type Target = DuckDb;

    fn deref(&self) -> &DuckDb {
        self.0.handle()
    }
}

impl axum::extract::FromRequestParts<AppState> for FleetDuckDb {
    type Rejection = AutumnError;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        autumn_web::fleet::resolve::<DuckDbEngine>(parts, state)
            .await
            .map(Self)
            .map_err(AutumnError::from)
    }
}

/// The configuration of a DuckDB fleet.
///
/// ```toml
/// [duckdb_fleet]
/// mode = "tenant"
/// root = "data/tenants"
/// path = "{bucket}/{tenant}.duckdb"
/// max_resident_bytes = 4294967296
/// ship_every_secs = 60
/// resident_bytes = 268435456
///
/// [duckdb_fleet.engine]
/// memory_limit = "256MB"
/// threads = 2
/// max_connections = 4
///
/// [duckdb_fleet.ownership]
/// owner = "host-a"
/// ```
///
/// The top keys are the Autumn fleet keys (`[database.fleet]`). `engine` has the `[duckdb]` keys,
/// except `path`. The variables `AUTUMN_DUCKDB_FLEET__ROOT` and
/// `AUTUMN_DUCKDB_FLEET__OWNERSHIP__OWNER` set `root` and `ownership.owner`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DuckDbFleetConfig {
    /// The Autumn fleet keys.
    pub fleet: DatabaseFleetConfig,
    /// The DuckDB configuration of each database. The fleet sets the path.
    pub engine: DuckDbConfig,
    /// The interval between two shipped generations.
    pub ship_every: Duration,
    /// The memory that the fleet counts for one open database.
    pub resident_bytes: u64,
}

impl DuckDbFleetConfig {
    /// Makes a configuration with `fleet`, the default engine and the default limits.
    #[must_use]
    pub fn new(fleet: DatabaseFleetConfig) -> Self {
        Self {
            fleet,
            engine: DuckDbConfig::default(),
            ship_every: DEFAULT_SHIP_EVERY,
            resident_bytes: DEFAULT_RESIDENT_BYTES,
        }
    }

    /// Reads `[section]` from the app files and the environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a file is not valid TOML or a value is not valid.
    pub fn resolve(section: &str) -> Result<Self, ConfigError> {
        autumn_web::dotenv::os_env_with_dotenv().map_or_else(
            |_| Self::resolve_with_env(section, &autumn_web::config::OsEnv),
            |env| Self::resolve_with_env(section, &env),
        )
    }

    /// Reads `[section]` with `env` as the environment.
    ///
    /// # Errors
    ///
    /// See [`resolve`](Self::resolve).
    pub fn resolve_with_env(section: &str, env: &dyn Env) -> Result<Self, ConfigError> {
        let mut table = layered_section(section, env)?;
        let prefix = env_prefix(section);
        if let Ok(root) = env.var(&format!("{prefix}__ROOT")) {
            table.insert("root".to_owned(), toml::Value::String(root));
        }
        if let Ok(owner) = env.var(&format!("{prefix}__OWNERSHIP__OWNER")) {
            let ownership = table
                .entry("ownership")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let toml::Value::Table(ownership) = ownership {
                ownership.insert("owner".to_owned(), toml::Value::String(owner));
            }
        }
        Self::from_table(section, table)
    }

    /// Reads a configuration from the TOML table of `[section]`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for an unknown key or a value that is not valid.
    pub fn from_table(section: &str, mut table: toml::Table) -> Result<Self, ConfigError> {
        let bad = |key: &str, err: &dyn std::fmt::Display| {
            ConfigError(format!("[{section}] {key}: {err}"))
        };
        let engine = match table.remove("engine") {
            None => DuckDbConfig::default(),
            Some(toml::Value::Table(engine)) => {
                if engine.contains_key("path") {
                    return Err(ConfigError(format!(
                        "{section}.engine.path must not be set: the fleet sets the path"
                    )));
                }
                toml::Value::Table(engine)
                    .try_into()
                    .map_err(|err| bad("engine", &err))?
            }
            Some(_) => return Err(ConfigError(format!("{section}.engine must be a table"))),
        };
        let ship_every = match table.remove("ship_every_secs") {
            None => DEFAULT_SHIP_EVERY,
            Some(value) => Duration::from_secs(
                value
                    .as_integer()
                    .and_then(|v| u64::try_from(v).ok())
                    .ok_or_else(|| bad("ship_every_secs", &"must be a whole number of seconds"))?,
            ),
        };
        let resident_bytes = match table.remove("resident_bytes") {
            None => DEFAULT_RESIDENT_BYTES,
            Some(value) => value
                .as_integer()
                .and_then(|v| u64::try_from(v).ok())
                .ok_or_else(|| bad("resident_bytes", &"must be a whole number"))?,
        };
        let fleet: DatabaseFleetConfig = toml::Value::Table(table)
            .try_into()
            .map_err(|err| bad("fleet", &err))?;
        let config = Self {
            fleet,
            engine,
            ship_every,
            resident_bytes,
        };
        config.validate_section(section)?;
        Ok(config)
    }

    /// Checks each value.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] that names the first key that is not valid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_section(DEFAULT_FLEET_SECTION)
    }

    fn validate_section(&self, section: &str) -> Result<(), ConfigError> {
        let fail = |key: &str, rule: &str| Err(ConfigError(format!("{section}.{key} {rule}")));
        if self.fleet.root.trim().is_empty() {
            return fail("root", "must be a directory");
        }
        if self.resident_bytes == 0 {
            return fail("resident_bytes", "must be 1 or more");
        }
        if self.ship_every.is_zero() {
            return fail("ship_every_secs", "must be 1 or more");
        }
        if self.engine.access_mode == AccessMode::ReadOnly {
            return fail(
                "engine.access_mode",
                "must not be `read_only`: the fleet writes a stamp",
            );
        }
        let mut engine = self.engine.clone();
        "fleet.duckdb".clone_into(&mut engine.path);
        engine.validate_section(&format!("{section}.engine"))
    }

    /// Makes the engine of this configuration.
    #[must_use]
    pub fn engine(&self) -> DuckDbEngine {
        DuckDbEngine::new(self.engine.clone())
            .ship_every(self.ship_every)
            .resident_bytes(self.resident_bytes)
    }
}

fn env_prefix(section: &str) -> String {
    let name: String = section
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("AUTUMN_{name}")
}

enum FleetSource {
    Section(String),
    Explicit(Box<DuckDbFleetConfig>),
}

/// The running parts of a fleet: the idle sweeper and the ownership loop.
struct FleetRuntime {
    fleet: DatabaseFleet<DuckDbEngine>,
    stop: CancellationToken,
    maintenance: tokio::task::JoinHandle<()>,
    ownership: Option<(CancellationToken, tokio::task::JoinHandle<()>)>,
}

impl FleetRuntime {
    async fn shutdown(self) {
        self.stop.cancel();
        let _ = self.maintenance.await;
        if let Some((stop, handle)) = self.ownership {
            stop.cancel();
            let _ = handle.await;
            if self.fleet.release_ownership().await {
                tracing::info!("the DuckDB fleet released its slot ranges");
            }
        }
    }
}

/// Makes a fleet of DuckDB databases and puts it in the app state.
///
/// ```rust,no_run
/// use autumn_plugin_duckdb::fleet::{DuckDbFleetPlugin, FleetDuckDb};
///
/// #[autumn_web::get("/count")]
/// async fn count(db: FleetDuckDb) -> autumn_web::AutumnResult<String> {
///     let rows = db.query("SELECT 1").fetch().await.map_err(|e| e.into_autumn())?;
///     Ok(rows.len().to_string())
/// }
///
/// # async fn run() {
/// autumn_web::app()
///     .plugin(DuckDbFleetPlugin::new())
///     .routes(autumn_web::routes![count])
///     .run()
///     .await;
/// # }
/// ```
#[must_use]
pub struct DuckDbFleetPlugin {
    source: FleetSource,
    setups: Vec<Setup>,
}

impl Default for DuckDbFleetPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl DuckDbFleetPlugin {
    /// Makes a plugin that reads `[duckdb_fleet]`.
    pub fn new() -> Self {
        Self {
            source: FleetSource::Section(DEFAULT_FLEET_SECTION.to_owned()),
            setups: Vec::new(),
        }
    }

    /// Reads `[section]` instead of `[duckdb_fleet]`.
    pub fn config_section(mut self, section: impl Into<String>) -> Self {
        self.source = FleetSource::Section(section.into());
        self
    }

    /// Uses `config` and reads no files or variables.
    pub fn config(mut self, config: DuckDbFleetConfig) -> Self {
        self.source = FleetSource::Explicit(Box::new(config));
        self
    }

    /// Adds Rust code that runs on each database at open, for example to create tables.
    ///
    /// Setup hooks run in order, before the plugin disables external access and locks the configuration.
    pub fn setup(
        mut self,
        hook: impl Fn(&Connection) -> duckdb::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.setups.push(Arc::new(hook));
        self
    }

    fn resolve(source: &FleetSource) -> Result<DuckDbFleetConfig, ConfigError> {
        match source {
            FleetSource::Section(section) => DuckDbFleetConfig::resolve(section),
            FleetSource::Explicit(config) => {
                config.validate()?;
                Ok((**config).clone())
            }
        }
    }
}

impl std::fmt::Debug for DuckDbFleetPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let source = match &self.source {
            FleetSource::Section(section) => section.as_str(),
            FleetSource::Explicit(_) => "(explicit)",
        };
        f.debug_struct("DuckDbFleetPlugin")
            .field("config", &source)
            .field("setups", &self.setups.len())
            .finish()
    }
}

/// Starts the fleet: ownership first, then the idle sweeper.
async fn start(
    state: &AppState,
    config: &DuckDbFleetConfig,
    engine: DuckDbEngine,
) -> Result<FleetRuntime, String> {
    let fleet = DatabaseFleet::with_engine(&config.fleet, engine).map_err(|e| e.to_string())?;
    let ownership = match &config.fleet.ownership {
        None => None,
        Some(settings) => {
            let store = autumn_web::fleet::ownership_store(state)
                .ok_or("fleet ownership needs a control database or a fleet_ownership_store")?;
            store.ensure_schema().await.map_err(|e| e.to_string())?;
            let ranges = settings
                .ranges()?
                .into_iter()
                .map(|(start, end)| SlotRange::new(start, end))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let owner = OwnerId::new(settings.owner.as_str()).map_err(|e| e.to_string())?;
            let ownership = Arc::new(
                FleetOwnership::new(
                    store,
                    owner,
                    ranges,
                    Duration::from_secs(settings.lease_secs),
                )
                .map_err(|e| e.to_string())?,
            );
            fleet
                .start_ownership(Arc::clone(&ownership))
                .await
                .map_err(|e| e.to_string())?;
            if let Err(err) = state.health_indicator_registry().register(
                OWNERSHIP_INDICATOR,
                autumn_web::actuator::IndicatorGroup::HealthOnly,
                ownership.indicator(),
            ) {
                tracing::warn!("{err}");
            }
            let stop = CancellationToken::new();
            let handle = fleet.spawn_ownership(stop.clone());
            Some((stop, handle))
        }
    };
    let stop = CancellationToken::new();
    let maintenance = fleet.spawn_maintenance(stop.clone());
    state.insert_extension(fleet.clone());
    Ok(FleetRuntime {
        fleet,
        stop,
        maintenance,
        ownership,
    })
}

impl Plugin for DuckDbFleetPlugin {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(FLEET_PLUGIN_NAME)
    }

    fn build(self, app: AppBuilder) -> AppBuilder {
        let Self { source, setups } = self;
        let resolved = Arc::new(Self::resolve(&source));
        let runtime: Arc<Mutex<Option<FleetRuntime>>> = Arc::default();
        let on_start = Arc::clone(&runtime);
        app.on_startup(move |state| {
            let resolved = Arc::clone(&resolved);
            let runtime = Arc::clone(&on_start);
            let setups = setups.clone();
            async move {
                let fail = |err: &dyn std::fmt::Display| {
                    AutumnError::internal_server_error_msg(format!("{FLEET_PLUGIN_NAME}: {err}"))
                };
                let config = resolved.as_ref().clone().map_err(|err| fail(&err))?;
                let mut engine = config.engine();
                engine.setups = setups;
                let started = start(&state, &config, engine)
                    .await
                    .map_err(|err| fail(&err))?;
                *runtime.lock().unwrap_or_else(PoisonError::into_inner) = Some(started);
                tracing::info!("the DuckDB fleet is ready");
                Ok(())
            }
        })
        .on_shutdown(move || {
            let runtime = runtime
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            async move {
                if let Some(runtime) = runtime {
                    runtime.shutdown().await;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests;
