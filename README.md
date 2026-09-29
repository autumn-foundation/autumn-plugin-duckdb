# autumn-plugin-duckdb

An [Autumn](https://github.com/autumn-foundation/autumn) plugin for [DuckDB](https://duckdb.org). Handlers run SQL with bound parameters and get typed rows.

- Bound parameters: prepared statements. One statement for each query.
- Typed rows: `serde` decodes rows into your structs, tuples or scalars.
- Limits: a timeout, a connection limit, and a row limit and a byte limit for each fetch. The plugin interrupts timed-out and dropped queries.
- Safe defaults: no file access, no extension downloads and a locked configuration after startup.
- Isolation: each call gets a new connection. No session state reaches the next call.
- Operations: a readiness check, Prometheus metrics and a `CHECKPOINT` at shutdown.
- Tests: use an in-memory database. The tests need no server.

## Install

```toml
[dependencies]
autumn-plugin-duckdb = "0.1"
```

```rust,ignore
use autumn_plugin_duckdb::{DuckDb, DuckDbPlugin, DuckDbResultExt as _};
use autumn_web::prelude::*;

#[derive(serde::Deserialize, serde::Serialize)]
struct Total {
    region: String,
    total: f64,
}

#[get("/totals/{year}")]
async fn totals(db: DuckDb, Path(year): Path<i32>) -> AutumnResult<Json<Vec<Total>>> {
    let rows = db
        .query("SELECT region, sum(amount) AS total FROM sales WHERE year = ? GROUP BY region")
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
            conn.execute_batch("CREATE TABLE IF NOT EXISTS sales (region TEXT, year INT, amount DOUBLE)")
        }))
        .routes(routes![totals])
        .run()
        .await;
}
```

An app can have one DuckDB plugin only. Use `ATTACH` in a setup hook for more databases.

The `bundled` feature is on by default. It builds DuckDB from source, so the first build takes some minutes. Turn it off to link a system `libduckdb`. The `parquet` and `json` features build those extensions into DuckDB. They also turn on `bundled`.

## Configuration

The plugin reads `[duckdb]` in `autumn.toml`. Profile sections and profile files override it. `AUTUMN_DUCKDB__<KEY>` variables override all files. A list variable has comma-separated items.

```toml
[duckdb]
path = "data/app.duckdb"          # Default: ":memory:".
access_mode = "automatic"         # "automatic", "read_only" or "read_write".
threads = 4                       # Optional. Default: the DuckDB default.
memory_limit = "2GB"              # Optional. Default: the DuckDB default.
max_connections = 8               # 1 to 1024. Calls in use at the same time.
timeout_ms = 30000                # 1 to 86400000. Includes the wait for a connection.
max_rows = 10000                  # More rows give an error.
max_result_bytes = 67108864       # More bytes of values give an error.
enable_external_access = false    # If true, SQL can read and write files and URLs.
allowed_directories = ["data/"]   # SQL can use these directories when external access is off.
autoinstall_extensions = false    # If true, DuckDB downloads known extensions.
autoload_extensions = false       # If true, DuckDB loads installed extensions.
lock_configuration = true         # If true, SQL cannot change settings after startup.
health_check = true               # Adds the `duckdb` readiness check.
checkpoint_on_shutdown = true     # Runs CHECKPOINT on a writable file at shutdown.

[duckdb.settings]                 # Other DuckDB options. TOML only.
default_order = "desc"
```

Do not put a key in `settings` that the plugin sets, for example `threads` or `enable_external_access`. Do not put `search_path` or `schema` there: they apply to one connection only. `read_only` needs a file. The plugin does not support MotherDuck paths (`md:`).

Change values in code with `DuckDbPlugin::configure`. Give the full configuration with `DuckDbPlugin::config`.

## Startup

The plugin opens the database in these steps:

1. Open with the access mode, the threads, the memory limit, the extension flags and `settings`.
2. Set `allowed_directories`.
3. Run the setup hooks, in order. They have full file access. Use them to create tables, load data or `LOAD` an extension.
4. Disable external access, unless `enable_external_access` is `true`.
5. Lock the configuration, unless `lock_configuration` is `false`.

A bad configuration or a failed setup hook stops the boot.

Setup hooks run on the root connection. Each call gets a new connection. So only database objects reach the calls: tables, views, macros, `ATTACH` and `LOAD`. Session state does not: `SET search_path`, `USE`, `SET VARIABLE`, temp tables and temp views.

## Queries

A call is one query or one `with_connection` closure.

| Method | Result |
|--------|--------|
| `execute()` | The changed row count. Use it for DDL, `INSERT`, `UPDATE` and `DELETE`. `INSERT ... RETURNING` and `CREATE TABLE ... AS` give 0. |
| `fetch()` | All rows as `Row` values. |
| `fetch_as::<T>()` | All rows as `T`. A struct reads by column name. A tuple reads by position. |
| `fetch_optional()`, `fetch_optional_as::<T>()` | The first row, if any. The plugin reads no more rows. |
| `fetch_one()`, `fetch_one_as::<T>()` | The first row. No row gives `DuckDbError::NotFound`. |

A row with one column also decodes into the column type: `fetch_one_as::<i64>()` for `SELECT count(*) ...`, or `fetch_one_as::<Vec<f64>>()` for `SELECT list(price) ...`.

`timeout(duration)` and `max_rows(n)` replace the configuration for one query.

DuckDB makes the full result before the plugin reads it. The row limit and the byte limit protect the app, not DuckDB. Use `LIMIT` in the SQL and `memory_limit` for DuckDB.

`DuckDb::with_connection` runs a closure on a new connection, on a blocking thread. Use it for the full `duckdb` API, for example a transaction or an appender. The timeout applies, but the row limit and the byte limit do not. DuckDB rolls back a transaction that the closure leaves open. At the timeout, the call returns. The closure continues until DuckDB stops its query. To return an app error, return `Ok(Err(app_error))` from the closure.

`DuckDb::open` opens a database without the plugin, for example in a test or a tool. Call `DuckDb::shutdown` when you do not need it.

### Parameters

Use `?` or `$1` in the SQL and `bind` for each value. The plugin compares the counts before the run. A mismatch gives `DuckDbError::ParameterCount`.

| Rust value | `Param` |
|------------|---------|
| `bool` | `Bool` |
| `i8` to `i64` | `Int` |
| `u8` to `u64` | `UInt` |
| `i128` | `HugeInt` |
| `u128` | `UHugeInt` |
| `isize`, `usize` | `Int`, `UInt` |
| `f32`, `f64` | `Float` |
| `&str`, `String` | `Text` |
| `Vec<u8>`, `&[u8]` | `Blob` |
| `None` | `Null` |

DuckDB casts text to the parameter type if the SQL gives the type, for example `d > ?` for a `DATE` column. In a function call, cast the parameter in the SQL: `year(?::DATE)`.

SQL text with more than one statement gives `DuckDbError::MultipleStatements`. SQL text that ends in an open literal or block comment gives `DuckDbError::OpenLiteral`. The plugin checks this before the call, because `duckdb-rs` runs each statement before the last one in `prepare`.

### Values

| DuckDB type | `Value` |
|-------------|---------|
| `TINYINT` to `BIGINT` | `Int` |
| `UTINYINT` to `UBIGINT` | `UInt` |
| `HUGEINT`, `UHUGEINT` | `HugeInt`, `UHugeInt` |
| `FLOAT`, `DOUBLE` | `Float` |
| `DECIMAL` | `Decimal` (exact text) |
| `VARCHAR`, `ENUM`, `UUID` | `Text` |
| `BLOB`, `BIT`, `GEOMETRY` | `Blob` |
| `DATE`, `TIME`, `TIMESTAMP` | `Date`, `Time`, `Timestamp` (ISO 8601 text) |
| `TIMESTAMPTZ` | `Timestamp` (ISO 8601 text in UTC, with `Z`) |
| `INTERVAL` | `Interval` |
| `LIST`, `ARRAY` | `List` |
| `STRUCT`, `MAP` | `Struct`, `Map` |
| `UNION` | the member value |

The plugin refuses `TIME_NS`, `TIMETZ`, `BIT`, `BIGNUM` and `VARIANT` columns with `DuckDbError::UnsupportedType`, because `duckdb-rs` loses data. Cast them to `VARCHAR` in the SQL. A `TIMESTAMPTZ` in a `UNION` has no `Z`.

A decimal decodes into `f64` or `String`. A decimal with no fraction also decodes into an integer type. A `HUGEINT` decodes into `i64` if it fits. `SUM` gives a `HUGEINT`.

A `Row` serializes as a JSON object. A map serializes as an object if each key is a scalar, or else as a list of `{key, value}` entries. JSON has no `NaN` or infinity: they become `null`.

## Errors

`or_http()` (from `DuckDbResultExt`) and `DuckDbError::into_autumn` give an `AutumnError` with the status in this table. The `?` operator also converts, but it always gives status 500.

| Error | Status |
|-------|--------|
| `NotFound` | 404 |
| `Database` with the class `Constraint` | 409 |
| `Timeout` | 504 |
| `Cancelled`, `ShuttingDown`, a write conflict | 503 |
| all other errors | 500 |

`is_retryable()` is `true` for a write conflict only. A timed-out query can have changed data before the interrupt.

The error text and the debug text show the DuckDB error class only, for example `Catalog`. `DuckDbError::detail` gives the full message. Do not show it to users. It can hold SQL text and key values. The plugin decode messages name the column but not the value. A custom `Deserialize` impl can put a value in its own message.

## Operations

- Readiness: the `duckdb` indicator runs `SELECT 1` on the root connection. It stays up when all connections are in use.
- Metrics: `duckdb_calls_started_total`, `duckdb_calls_total{outcome}`, `duckdb_calls_open` and `duckdb_rows_returned_total`.
- Shutdown: calls work while Autumn drains the requests. Then the shutdown hook refuses new calls, interrupts open calls and runs `CHECKPOINT` on a writable file. If Autumn ends the process before the hook, the WAL keeps the data.
- Logs: the plugin does not log SQL text, parameter values or DuckDB messages.

## Compatibility

- `autumn-web` 0.7.
- DuckDB 1.5 through the `duckdb` crate `~1.10505`. `duckdb-rs` puts each DuckDB release in a minor version, so an upgrade is a new minor version of this crate.
- Rust 1.88 or later.

## License

Apache-2.0.
