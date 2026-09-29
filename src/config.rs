//! The `[duckdb]` section of `autumn.toml`.
//!
//! # Contract
//!
//! Each layer overrides the layers before it:
//!
//! 1. The defaults.
//! 2. `[duckdb]` in `autumn.toml`.
//! 3. `[profile.<name>.duckdb]` in `autumn.toml`.
//! 4. `[duckdb]` in `autumn-<name>.toml`.
//! 5. `AUTUMN_DUCKDB__<KEY>` variables. `AUTUMN_DUCKDB__MAX_ROWS` sets `max_rows`.
//!    A list variable has comma-separated items. `settings` has no variables.
//!
//! The result must pass [`DuckDbConfig::validate`]. Unknown keys are errors.
//!
//! ```toml
//! [duckdb]
//! path = "data/app.duckdb"
//! access_mode = "read_only"
//! threads = 4
//! memory_limit = "2GB"
//! max_connections = 8
//! timeout_ms = 30000
//! max_rows = 10000
//! max_result_bytes = 67108864
//! allowed_directories = ["data/"]
//!
//! [duckdb.settings]
//! default_order = "desc"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use autumn_web::config::Env;
use serde::{Deserialize, Serialize};

/// The default section name.
pub const DEFAULT_SECTION: &str = "duckdb";

/// The path of an in-memory database.
pub const IN_MEMORY: &str = ":memory:";

/// A configuration that is not valid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub(crate) String);

/// How the plugin opens the database file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AccessMode {
    /// DuckDB selects the mode. It is read-write for a file.
    #[default]
    Automatic,
    /// Reads only. Many processes can open the file.
    ReadOnly,
    /// Reads and writes. One process can open the file.
    ReadWrite,
}

/// The plugin settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
#[non_exhaustive]
#[allow(clippy::struct_excessive_bools, reason = "each bool is one TOML key")]
pub struct DuckDbConfig {
    /// The database file. `:memory:` gives an in-memory database.
    pub path: String,
    /// How the plugin opens the file.
    pub access_mode: AccessMode,
    /// The DuckDB worker threads. `None` uses the DuckDB default.
    pub threads: Option<u32>,
    /// The DuckDB memory limit, for example `"2GB"`. `None` uses the DuckDB default.
    pub memory_limit: Option<String>,
    /// The most connections that can run calls at the same time.
    pub max_connections: usize,
    /// The call timeout in milliseconds. It includes the wait for a connection.
    pub timeout_ms: u64,
    /// The most rows that one query can return.
    pub max_rows: usize,
    /// The most bytes of values that one query can return.
    pub max_result_bytes: usize,
    /// If `true`, SQL can read and write files and URLs.
    pub enable_external_access: bool,
    /// Directories that SQL can use when external access is off.
    pub allowed_directories: Vec<String>,
    /// If `true`, DuckDB downloads a known extension when SQL needs it.
    pub autoinstall_extensions: bool,
    /// If `true`, DuckDB loads an installed extension when SQL needs it.
    pub autoload_extensions: bool,
    /// If `true`, SQL cannot change the DuckDB configuration after startup.
    pub lock_configuration: bool,
    /// If `true`, the plugin adds a readiness check.
    pub health_check: bool,
    /// If `true`, the plugin runs `CHECKPOINT` at shutdown on a writable file.
    pub checkpoint_on_shutdown: bool,
    /// Other DuckDB options, for example `default_order = "desc"`.
    ///
    /// The keys that the plugin sets are not allowed here.
    pub settings: BTreeMap<String, String>,
}

impl Default for DuckDbConfig {
    fn default() -> Self {
        Self {
            path: IN_MEMORY.to_owned(),
            access_mode: AccessMode::Automatic,
            threads: None,
            memory_limit: None,
            max_connections: 8,
            timeout_ms: 30_000,
            max_rows: 10_000,
            max_result_bytes: 64 * 1024 * 1024,
            enable_external_access: false,
            allowed_directories: Vec::new(),
            autoinstall_extensions: false,
            autoload_extensions: false,
            lock_configuration: true,
            health_check: true,
            checkpoint_on_shutdown: true,
            settings: BTreeMap::new(),
        }
    }
}

/// The type of a configuration leaf, for environment values.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    Integer,
    Bool,
    List,
}

/// Each leaf key and its type.
const LEAVES: &[(&str, Kind)] = &[
    ("path", Kind::Text),
    ("access_mode", Kind::Text),
    ("threads", Kind::Integer),
    ("memory_limit", Kind::Text),
    ("max_connections", Kind::Integer),
    ("timeout_ms", Kind::Integer),
    ("max_rows", Kind::Integer),
    ("max_result_bytes", Kind::Integer),
    ("enable_external_access", Kind::Bool),
    ("allowed_directories", Kind::List),
    ("autoinstall_extensions", Kind::Bool),
    ("autoload_extensions", Kind::Bool),
    ("lock_configuration", Kind::Bool),
    ("health_check", Kind::Bool),
    ("checkpoint_on_shutdown", Kind::Bool),
];

/// The DuckDB options that the plugin sets. `settings` cannot set them.
pub(crate) const MANAGED_SETTINGS: &[&str] = &[
    "access_mode",
    "threads",
    "worker_threads",
    "memory_limit",
    "max_memory",
    "enable_external_access",
    "allowed_directories",
    "allowed_paths",
    "autoinstall_known_extensions",
    "autoload_known_extensions",
    "allow_unsigned_extensions",
    "lock_configuration",
];

/// DuckDB options for one connection. Each call has a new connection, so they have no effect.
const SESSION_SETTINGS: &[&str] = &["search_path", "schema"];

/// The largest call timeout: one day.
const MAX_TIMEOUT_MS: u64 = 86_400_000;

/// The largest connection count.
const MAX_CONNECTIONS: usize = 1024;

impl DuckDbConfig {
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
        let (selected, profile) = active_profile(env);
        let mut merged = toml::Table::new();
        if let Some(base) = read_toml(&config_file("autumn.toml", env))? {
            merge_section(&mut merged, base.get(section), section)?;
            for name in inline_profile_names(&profile) {
                let inline = base
                    .get("profile")
                    .and_then(|p| p.get(name))
                    .and_then(|p| p.get(section));
                merge_section(&mut merged, inline, section)?;
            }
        }
        for name in autumn_web::config::profile_override_file_lookup_names(&profile, &selected) {
            if let Some(file) = read_toml(&config_file(&format!("autumn-{name}.toml"), env))? {
                merge_section(&mut merged, file.get(section), section)?;
                break;
            }
        }
        apply_env(&mut merged, section, env)?;
        let config: Self = toml::Value::Table(merged)
            .try_into()
            .map_err(|err| ConfigError(format!("[{section}]: {err}")))?;
        config.validate_section(section)?;
        Ok(config)
    }

    /// Checks each value.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] that names the first key that is not valid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_section(DEFAULT_SECTION)
    }

    /// Checks each value. The errors name keys in `section`.
    pub(crate) fn validate_section(&self, section: &str) -> Result<(), ConfigError> {
        let fail = |key: &str, rule: &str| Err(ConfigError(format!("{section}.{key} {rule}")));
        let lower = self.path.to_ascii_lowercase();
        if self.path.trim().is_empty() {
            return fail("path", "must be a file path or `:memory:`");
        }
        if lower.starts_with("md:") || lower.starts_with("motherduck:") {
            return fail("path", "must be a local file: MotherDuck is not supported");
        }
        if self.access_mode == AccessMode::ReadOnly && self.is_in_memory() {
            return fail("access_mode", "must not be `read_only` for `:memory:`");
        }
        if self.threads == Some(0) {
            return fail("threads", "must be 1 or more");
        }
        if self
            .memory_limit
            .as_deref()
            .is_some_and(|v| v.trim().is_empty())
        {
            return fail("memory_limit", "must not be empty");
        }
        if !(1..=MAX_CONNECTIONS).contains(&self.max_connections) {
            return fail("max_connections", "must be from 1 to 1024");
        }
        if !(1..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return fail("timeout_ms", "must be from 1 to 86400000 (one day)");
        }
        if self.max_rows == 0 {
            return fail("max_rows", "must be 1 or more");
        }
        if self.max_result_bytes == 0 {
            return fail("max_result_bytes", "must be 1 or more");
        }
        if self.allowed_directories.iter().any(|d| d.trim().is_empty()) {
            return fail("allowed_directories", "must not have an empty item");
        }
        for key in self.settings.keys() {
            let name_ok = key.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
                && key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if !name_ok {
                return fail(
                    "settings",
                    "keys must be `a-z 0-9 _` and start with a letter or `_`",
                );
            }
            if SESSION_SETTINGS.contains(&key.as_str()) {
                return fail(
                    "settings",
                    &format!("must not set `{key}`: it applies to one connection only"),
                );
            }
            if MANAGED_SETTINGS.contains(&key.as_str()) {
                return fail(
                    "settings",
                    &format!("must not set `{key}`: the plugin sets it"),
                );
            }
        }
        Ok(())
    }

    /// The call timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    /// Returns `true` for an in-memory database.
    #[must_use]
    pub fn is_in_memory(&self) -> bool {
        self.path == IN_MEMORY
    }
}

/// Gives the selected profile text and the normalized profile, as Autumn does.
fn active_profile(env: &dyn Env) -> (String, String) {
    let selected = ["AUTUMN_ENV", "AUTUMN_PROFILE"]
        .iter()
        .filter_map(|key| env.var(key).ok())
        .map(|value| value.trim().to_owned())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let release = env.var("AUTUMN_IS_DEBUG").is_ok_and(|v| v == "0");
            if release { "prod" } else { "dev" }.to_owned()
        });
    let profile =
        autumn_web::config::normalize_profile_name(&selected).unwrap_or_else(|| "dev".to_owned());
    (selected, profile)
}

/// The inline profile names to read, in order. The canonical name is last.
fn inline_profile_names(profile: &str) -> Vec<&str> {
    match profile {
        "prod" => vec!["production", "prod"],
        "dev" => vec!["development", "dev"],
        other => vec![other],
    }
}

/// Finds a config file in `AUTUMN_MANIFEST_DIR`, or else in the working directory.
fn config_file(name: &str, env: &dyn Env) -> PathBuf {
    env.var("AUTUMN_MANIFEST_DIR")
        .ok()
        .map(|dir| Path::new(&dir).join(name))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn read_toml(path: &Path) -> Result<Option<toml::Table>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<toml::Table>()
            .map(Some)
            .map_err(|err| ConfigError(format!("{}: {err}", path.display()))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ConfigError(format!("{}: {err}", path.display()))),
    }
}

fn merge_section(
    into: &mut toml::Table,
    layer: Option<&toml::Value>,
    section: &str,
) -> Result<(), ConfigError> {
    match layer {
        None => Ok(()),
        Some(toml::Value::Table(table)) => {
            deep_merge(into, table);
            Ok(())
        }
        Some(_) => Err(ConfigError(format!("[{section}] must be a table"))),
    }
}

fn deep_merge(into: &mut toml::Table, layer: &toml::Table) {
    for (key, value) in layer {
        match (into.get_mut(key), value) {
            (Some(toml::Value::Table(old)), toml::Value::Table(new)) => deep_merge(old, new),
            _ => {
                into.insert(key.clone(), value.clone());
            }
        }
    }
}

fn apply_env(into: &mut toml::Table, section: &str, env: &dyn Env) -> Result<(), ConfigError> {
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
    for (key, kind) in LEAVES {
        let name = format!("AUTUMN_{name}__{}", key.to_ascii_uppercase());
        let Ok(raw) = env.var(&name) else {
            continue;
        };
        let bad = || ConfigError(format!("{name}: can not read {raw:?}"));
        let value = match kind {
            Kind::Text => toml::Value::String(raw.clone()),
            Kind::Integer => {
                let value: i64 = raw.trim().parse().map_err(|_| bad())?;
                if value < 0 {
                    return Err(bad());
                }
                toml::Value::Integer(value)
            }
            Kind::Bool => match raw.trim() {
                "true" | "1" => toml::Value::Boolean(true),
                "false" | "0" => toml::Value::Boolean(false),
                _ => return Err(bad()),
            },
            Kind::List => toml::Value::Array(
                raw.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| toml::Value::String(item.to_owned()))
                    .collect(),
            ),
        };
        into.insert((*key).to_owned(), value);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
