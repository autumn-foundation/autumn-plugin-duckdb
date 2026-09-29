#![allow(
    clippy::field_reassign_with_default,
    reason = "each test changes one key of the defaults"
)]

use std::path::Path;

use autumn_web::config::MockEnv;

use super::*;

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

fn env_for(dir: &Path) -> MockEnv {
    MockEnv::new().with("AUTUMN_MANIFEST_DIR", dir.to_str().unwrap())
}

fn resolve(dir: &Path, env: &MockEnv) -> Result<DuckDbConfig, ConfigError> {
    let _ = dir;
    DuckDbConfig::resolve_with_env("duckdb", env)
}

#[test]
fn defaults_are_safe() {
    let config = DuckDbConfig::default();
    assert_eq!(config.path, IN_MEMORY);
    assert!(config.is_in_memory());
    assert_eq!(config.access_mode, AccessMode::Automatic);
    assert_eq!(config.threads, None);
    assert_eq!(config.memory_limit, None);
    assert_eq!(config.max_connections, 8);
    assert_eq!(config.timeout(), Duration::from_secs(30));
    assert_eq!(config.max_rows, 10_000);
    assert_eq!(config.max_result_bytes, 64 * 1024 * 1024);
    assert!(!config.enable_external_access);
    assert!(config.allowed_directories.is_empty());
    assert!(!config.autoinstall_extensions);
    assert!(!config.autoload_extensions);
    assert!(config.lock_configuration);
    assert!(config.health_check);
    assert!(config.checkpoint_on_shutdown);
    assert!(config.settings.is_empty());
    config.validate().unwrap();
}

#[test]
fn no_file_gives_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config = resolve(dir.path(), &env_for(dir.path())).unwrap();
    assert_eq!(config, DuckDbConfig::default());
}

#[test]
fn reads_the_section_from_autumn_toml() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "autumn.toml",
        r#"
[duckdb]
path = "data/app.duckdb"
access_mode = "read_only"
threads = 2
memory_limit = "1GB"
max_connections = 3
timeout_ms = 5000
max_rows = 10
max_result_bytes = 1000
enable_external_access = true
allowed_directories = ["data/"]
autoinstall_extensions = true
autoload_extensions = true
lock_configuration = false
health_check = false
checkpoint_on_shutdown = false

[duckdb.settings]
default_order = "desc"
"#,
    );
    let config = resolve(dir.path(), &env_for(dir.path())).unwrap();
    assert_eq!(config.path, "data/app.duckdb");
    assert!(!config.is_in_memory());
    assert_eq!(config.access_mode, AccessMode::ReadOnly);
    assert_eq!(config.threads, Some(2));
    assert_eq!(config.memory_limit.as_deref(), Some("1GB"));
    assert_eq!(config.max_connections, 3);
    assert_eq!(config.timeout_ms, 5000);
    assert_eq!(config.max_rows, 10);
    assert_eq!(config.max_result_bytes, 1000);
    assert!(config.enable_external_access);
    assert_eq!(config.allowed_directories, vec!["data/".to_owned()]);
    assert!(config.autoinstall_extensions);
    assert!(config.autoload_extensions);
    assert!(!config.lock_configuration);
    assert!(!config.health_check);
    assert!(!config.checkpoint_on_shutdown);
    assert_eq!(config.settings["default_order"], "desc");
}

#[test]
fn reads_a_custom_section() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[analytics]\nmax_rows = 5\n");
    let config = DuckDbConfig::resolve_with_env("analytics", &env_for(dir.path())).unwrap();
    assert_eq!(config.max_rows, 5);
}

#[test]
fn inline_profile_overrides_the_base() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "autumn.toml",
        "[duckdb]\npath = \"dev.duckdb\"\nmax_rows = 5\n[profile.prod.duckdb]\npath = \"prod.duckdb\"\n",
    );
    let env = env_for(dir.path()).with("AUTUMN_ENV", "production");
    let config = resolve(dir.path(), &env).unwrap();
    assert_eq!(config.path, "prod.duckdb");
    assert_eq!(config.max_rows, 5);
}

#[test]
fn profile_file_overrides_the_inline_profile() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "autumn.toml",
        "[duckdb]\npath = \"base\"\n[profile.staging.duckdb]\npath = \"inline\"\n",
    );
    write(dir.path(), "autumn-staging.toml", "[duckdb]\npath = \"file\"\n");
    let env = env_for(dir.path()).with("AUTUMN_PROFILE", "staging");
    assert_eq!(resolve(dir.path(), &env).unwrap().path, "file");
}

#[test]
fn settings_merge_across_layers() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "autumn.toml",
        "[duckdb.settings]\na = \"1\"\nb = \"2\"\n[profile.dev.duckdb.settings]\nb = \"3\"\n",
    );
    let config = resolve(dir.path(), &env_for(dir.path())).unwrap();
    assert_eq!(config.settings["a"], "1");
    assert_eq!(config.settings["b"], "3");
}

#[test]
fn environment_overrides_the_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "autumn.toml",
        "[duckdb]\npath = \"base\"\nmax_rows = 5\n",
    );
    let env = env_for(dir.path())
        .with("AUTUMN_DUCKDB__PATH", "env.duckdb")
        .with("AUTUMN_DUCKDB__MAX_ROWS", "7")
        .with("AUTUMN_DUCKDB__HEALTH_CHECK", "false")
        .with("AUTUMN_DUCKDB__ACCESS_MODE", "read_write")
        .with("AUTUMN_DUCKDB__ALLOWED_DIRECTORIES", "a/, b/ ,");
    let config = resolve(dir.path(), &env).unwrap();
    assert_eq!(config.path, "env.duckdb");
    assert_eq!(config.max_rows, 7);
    assert!(!config.health_check);
    assert_eq!(config.access_mode, AccessMode::ReadWrite);
    assert_eq!(config.allowed_directories, vec!["a/".to_owned(), "b/".to_owned()]);
}

#[test]
fn a_numeric_memory_limit_from_the_environment_stays_text() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_for(dir.path()).with("AUTUMN_DUCKDB__MEMORY_LIMIT", "1000");
    let config = resolve(dir.path(), &env).unwrap();
    assert_eq!(config.memory_limit.as_deref(), Some("1000"));
}

#[test]
fn environment_values_parse_by_type() {
    let dir = tempfile::tempdir().unwrap();
    let base = env_for(dir.path());
    let config = resolve(
        dir.path(),
        &base
            .clone()
            .with("AUTUMN_DUCKDB__HEALTH_CHECK", "0")
            .with("AUTUMN_DUCKDB__LOCK_CONFIGURATION", "1"),
    )
    .unwrap();
    assert!(!config.health_check);
    assert!(config.lock_configuration);
    for (key, value) in [
        ("AUTUMN_DUCKDB__HEALTH_CHECK", "yes"),
        ("AUTUMN_DUCKDB__MAX_ROWS", "many"),
        ("AUTUMN_DUCKDB__MAX_ROWS", "-1"),
    ] {
        let err = resolve(dir.path(), &base.clone().with(key, value)).unwrap_err();
        assert!(err.to_string().contains(key), "{key}: {err}");
    }
}

#[test]
fn unknown_keys_fail() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[duckdb]\nmax_row = 1\n");
    assert!(resolve(dir.path(), &env_for(dir.path())).is_err());
}

#[test]
fn a_bad_access_mode_fails() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[duckdb]\naccess_mode = \"write_only\"\n");
    assert!(resolve(dir.path(), &env_for(dir.path())).is_err());
}

#[test]
fn a_section_that_is_not_a_table_fails() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "duckdb = 5\n");
    assert!(resolve(dir.path(), &env_for(dir.path())).is_err());
}

#[test]
fn bad_toml_fails() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[duckdb\n");
    assert!(resolve(dir.path(), &env_for(dir.path())).is_err());
}

#[test]
fn a_config_path_that_is_a_directory_fails() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("autumn.toml")).unwrap();
    assert!(resolve(dir.path(), &env_for(dir.path())).is_err());
}

#[test]
fn resolve_validates_the_result() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[duckdb]\nmax_rows = 0\n");
    let err = resolve(dir.path(), &env_for(dir.path())).unwrap_err();
    assert!(err.to_string().contains("duckdb.max_rows"), "{err}");
}

#[test]
fn errors_name_a_custom_section() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[analytics]\nmax_rows = 0\n");
    let err = DuckDbConfig::resolve_with_env("analytics", &env_for(dir.path())).unwrap_err();
    assert!(err.to_string().contains("analytics.max_rows"), "{err}");
}

#[test]
fn a_section_name_with_a_dash_gives_a_valid_variable() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_for(dir.path()).with("AUTUMN_DUCK_ONE__MAX_ROWS", "4");
    let config = DuckDbConfig::resolve_with_env("duck-one", &env).unwrap();
    assert_eq!(config.max_rows, 4);
}

#[test]
fn only_the_first_profile_file_is_read() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn-prod.toml", "[duckdb]\npath = \"prod\"\n");
    write(
        dir.path(),
        "autumn-production.toml",
        "[duckdb]\npath = \"production\"\nmax_rows = 3\n",
    );
    let env = env_for(dir.path()).with("AUTUMN_ENV", "prod");
    let config = resolve(dir.path(), &env).unwrap();
    assert_eq!(config.path, "prod");
    assert_eq!(config.max_rows, 10_000);
}

#[test]
fn a_release_build_uses_the_prod_profile() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[profile.prod.duckdb]\npath = \"p\"\n");
    let env = env_for(dir.path()).with("AUTUMN_IS_DEBUG", "0");
    assert_eq!(resolve(dir.path(), &env).unwrap().path, "p");
}

#[test]
fn the_canonical_inline_profile_wins_over_its_alias() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "autumn.toml",
        "[profile.production.duckdb]\npath = \"alias\"\n[profile.prod.duckdb]\npath = \"canonical\"\n",
    );
    let env = env_for(dir.path()).with("AUTUMN_ENV", "prod");
    assert_eq!(resolve(dir.path(), &env).unwrap().path, "canonical");
}

#[test]
fn an_environment_path_through_a_value_fails() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "autumn.toml", "[duckdb]\nsettings = 5\n");
    assert!(resolve(dir.path(), &env_for(dir.path())).is_err());
}

fn invalid(change: impl FnOnce(&mut DuckDbConfig)) -> String {
    let mut config = DuckDbConfig::default();
    change(&mut config);
    config.validate().unwrap_err().to_string()
}

#[test]
fn validation_names_the_bad_key() {
    assert!(invalid(|c| c.path = String::new()).contains("duckdb.path"));
    assert!(invalid(|c| c.path = "  ".into()).contains("duckdb.path"));
    assert!(invalid(|c| c.path = "md:my_db".into()).contains("duckdb.path"));
    assert!(invalid(|c| c.path = "MotherDuck:x".into()).contains("duckdb.path"));
    assert!(invalid(|c| c.access_mode = AccessMode::ReadOnly).contains("duckdb.access_mode"));
    assert!(invalid(|c| c.threads = Some(0)).contains("duckdb.threads"));
    assert!(invalid(|c| c.memory_limit = Some(" ".into())).contains("duckdb.memory_limit"));
    assert!(invalid(|c| c.max_connections = 0).contains("duckdb.max_connections"));
    assert!(invalid(|c| c.max_connections = 1025).contains("duckdb.max_connections"));
    assert!(invalid(|c| c.timeout_ms = 0).contains("duckdb.timeout_ms"));
    assert!(invalid(|c| c.timeout_ms = 86_400_001).contains("duckdb.timeout_ms"));
    assert!(invalid(|c| c.max_rows = 0).contains("duckdb.max_rows"));
    assert!(invalid(|c| c.max_result_bytes = 0).contains("duckdb.max_result_bytes"));
    assert!(
        invalid(|c| c.allowed_directories = vec![String::new()])
            .contains("duckdb.allowed_directories")
    );
}

#[test]
fn settings_refuse_managed_and_bad_keys() {
    for key in [
        "threads",
        "memory_limit",
        "max_memory",
        "access_mode",
        "enable_external_access",
        "allowed_directories",
        "autoinstall_known_extensions",
        "autoload_known_extensions",
        "allow_unsigned_extensions",
        "lock_configuration",
        "Threads",
        "",
        "bad key",
        "a;b",
    ] {
        let message = invalid(|c| {
            c.settings.insert(key.to_owned(), "1".to_owned());
        });
        assert!(message.contains("duckdb.settings"), "{key}: {message}");
    }
}

#[test]
fn validation_boundaries_pass() {
    let mut config = DuckDbConfig::default();
    config.path = "data/app.duckdb".into();
    config.access_mode = AccessMode::ReadOnly;
    config.threads = Some(1);
    config.memory_limit = Some("512MB".into());
    config.max_connections = 1024;
    config.timeout_ms = 86_400_000;
    config.max_rows = 1;
    config.max_result_bytes = 1;
    config.allowed_directories = vec!["data/".into()];
    config
        .settings
        .insert("default_order".into(), "desc".into());
    config.validate().unwrap();
    config.max_connections = 1;
    config.timeout_ms = 1;
    config.validate().unwrap();
}

fn leaf_paths(prefix: &str, table: &toml::Table, out: &mut Vec<String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => leaf_paths(&path, inner, out),
            _ => out.push(path),
        }
    }
}

#[test]
fn each_field_but_settings_has_an_environment_variable() {
    let full = DuckDbConfig {
        threads: Some(1),
        memory_limit: Some(String::new()),
        ..DuckDbConfig::default()
    };
    let toml::Value::Table(table) = toml::Value::try_from(&full).unwrap() else {
        panic!("a config must serialize as a table");
    };
    let mut fields = Vec::new();
    leaf_paths("", &table, &mut fields);
    fields.sort();
    let mut leaves: Vec<String> = LEAVES.iter().map(|(path, _)| (*path).to_owned()).collect();
    leaves.sort();
    assert_eq!(fields, leaves);
}
