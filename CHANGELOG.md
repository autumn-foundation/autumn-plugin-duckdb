# Changelog

## 0.1.0

- `DuckDbPlugin` and the `DuckDb` extractor. `DuckDb::open` and `DuckDb::shutdown` for use without the plugin.
- `DuckDbQuery` with bound parameters, `execute`, `fetch`, `fetch_as`, `fetch_optional`, `fetch_optional_as`, `fetch_one` and `fetch_one_as`.
- Per-query `timeout` and `max_rows`.
- `DuckDb::with_connection` for the full `duckdb` API.
- `Value`, `Row` and `Param` types. Serde decoding of rows.
- A timeout and a connection limit for each call. A row limit and a byte limit for each fetch.
- The plugin interrupts timed-out, dropped and open-at-shutdown calls.
- Each call gets a new connection, so no session state leaks between calls.
- Safe defaults: no external access, no extension auto-install or auto-load, a locked configuration.
- Refusal of SQL with more than one statement.
- Setup hooks that run before the plugin disables external access and locks the configuration.
- A readiness check, Prometheus metrics and a `CHECKPOINT` at shutdown.
- Features: `bundled` (default), `parquet` and `json`.
