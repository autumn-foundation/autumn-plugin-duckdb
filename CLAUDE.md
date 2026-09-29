# CLAUDE.md

Guidance for agents that work on this crate.

## What this crate is

`autumn-plugin-duckdb` is an Autumn plugin. Autumn is `autumn-web` 0.7. Handlers run DuckDB queries through the `DuckDb` extractor. Read `docs/planning.md` before a design change.

## Commands

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo check --locked --lib
cargo test --locked --all-targets --all-features
cargo +1.88.0 check --locked --all-targets --all-features
cargo test --locked --doc --all-features
RUSTDOCFLAGS=-D\ warnings cargo doc --locked --no-deps --all-features
cargo llvm-cov --locked --all-features --ignore-filename-regex '/tests\.rs$' --fail-under-lines 90
```

The MSRV is 1.88. The first build compiles DuckDB from C++ source. It takes some minutes.

## Architecture

| Module | Kind | Job |
|--------|------|-----|
| `config` | pure | `DuckDbConfig`, layering and validation. |
| `param` | pure | `Param`, the bound value type. |
| `statement` | pure | Counts the statements in SQL text. |
| `temporal` | pure | Date, time and timestamp text. |
| `value` | pure | `Value` and `Row`. Conversion from DuckDB values. Result size. |
| `decode` | pure | Serde decoding of `Value` and `Row`. |
| `error` | data | `DuckDbError`, the error class and the HTTP status map. |
| `pool` | glue | The connection pool and the open steps. |
| `client` | glue | `DuckDb` and `DuckDbQuery`: run, timeout, interrupt, limits. |
| `plugin` | glue | `DuckDbPlugin` and the extractor. |
| `health` | glue | The readiness check. |
| `metrics` | glue | Counters and the metrics source. |

Each pure module has a `# Contract` doc section. Change the contract first. Then change the tests. Then change the code.

## Rules

- Work red, green, refactor. Write the failing test first.
- Put unit tests in `src/<module>/tests.rs`. The coverage gate ignores these files.
- Production code has no `unwrap`, `expect` or `panic`. Clippy denies them outside tests.
- Never put a value into SQL text. Bind it with `Param`.
- Never log SQL text, parameter values or DuckDB messages. A DuckDB message can hold a key value.
- Each DuckDB call runs on a blocking thread. Never call DuckDB on an async worker thread.
- Each call has a deadline. At the deadline, the plugin interrupts the query.
- External access, extension auto-install and auto-load are off by default. The plugin locks the configuration after startup.
- Each call gets a new connection. Never reuse a connection: session state leaks.
- Never log or show `DuckDbError::detail`. The debug text must also hide it.
- Metric names must not start with `autumn_`.
- No test uses the network. Tests use in-memory DuckDB or a temporary file.

## Test notes

- `TestApp` runs startup hooks but not shutdown hooks. `plugin::tests` tests the shutdown hook.
- `AppState::begin_shutdown_for_test` marks the shutdown. Calls must still work: Autumn drains the requests before the hooks.
- A slow query for timeout tests: `SELECT count(*) FROM range(100000) a, range(10000000) b`.
- Do not use sleeps to order events in tests. Use the `hold` helper in `client::tests`: it holds a connection until a signal.

## Documentation style

Write docs and comments in ASD-STE100 style: short sentences, active voice, simple present tense, one instruction per sentence. Keep instructions at 20 words or fewer and descriptions at 25 words or fewer.
