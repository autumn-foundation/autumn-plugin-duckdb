# Changelog

## 0.1.0

- `DuckDbPlugin` and the `DuckDb` extractor.
- `DuckDbQuery` with bound parameters, `execute`, `fetch`, `fetch_as`, `fetch_optional` and `fetch_one_as`.
- `DuckDb::with_connection` for the full `duckdb` API.
- A timeout, a row limit, a byte limit and a connection limit for each call.
- The plugin interrupts timed-out, dropped and open-at-shutdown calls.
- Safe defaults: no external access, no extension auto-install or auto-load, a locked configuration.
- Setup hooks that run before the limits apply.
- A readiness check, Prometheus metrics and a `CHECKPOINT` at shutdown.
