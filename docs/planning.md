# Planning

This document records the plan for `autumn-plugin-duckdb`. It uses three methods: brainstorming, reverse brainstorming and six thinking hats. The last sections give the decisions, the architecture and the TDD plan.

## Goal

An Autumn app runs DuckDB SQL from a handler. The app does one install step. The plugin keeps time, memory and file access in limits.

## 1. Brainstorming

Write all ideas first. Do not judge them in this step.

1. A handler extractor `DuckDb` that gives a database handle.
2. A fluent query builder: `duckdb.query(sql).bind(v).fetch_as::<T>()`.
3. Bind parameters with DuckDB prepared statements.
4. Decode rows into `serde` structs by column name.
5. A `Value` type for each DuckDB type, with lists, structs and maps.
6. Date, time and timestamp values as ISO 8601 text.
7. A connection pool. DuckDB calls block, so they run on blocking threads.
8. A query timeout that interrupts the query in DuckDB.
9. Interrupt the query when the caller drops the future.
10. Interrupt all open queries at app shutdown.
11. A `CHECKPOINT` at shutdown for a file database.
12. A row limit and a byte limit for each result.
13. Read `[duckdb]` in `autumn.toml`, with profiles and `AUTUMN_DUCKDB__*` variables.
14. An in-memory database or a file database. A read-only mode.
15. DuckDB settings: `threads`, `memory_limit` and other keys.
16. Disable external access by default. Allow a list of directories.
17. Disable extension auto-install and auto-load by default.
18. Lock the DuckDB configuration after startup.
19. A setup hook: Rust code that runs on the database before requests.
20. `with_connection`: run Rust code on a pooled connection, for example a transaction or an appender.
21. A readiness check that runs `SELECT 1`.
22. Prometheus metrics: queries by outcome, open queries, rows returned.
23. Map DuckDB error classes to HTTP status codes.
24. Arrow and Polars results.
25. Streaming rows.
26. MotherDuck (cloud DuckDB) support.
27. A migration runner.
28. A test fake.
29. More than one database in one app.

## 2. Reverse brainstorming

Question: "How can we make this plugin fail?" Each answer gives a countermeasure.

| How to make it fail | Countermeasure |
|---------------------|----------------|
| Put user input into the SQL text. | Bind values with prepared statements. Do not format SQL. |
| Send two statements in one string. `duckdb-rs` runs all but the last statement in `prepare`. | Count the statements outside literals and comments. Refuse more than one. DuckDB is the test oracle. |
| Send the wrong number of parameters. | Compare the parameter count before the run. Give a typed error. |
| Read `/etc/passwd` with `read_csv` through an injected query. | Disable external access by default. Allow only listed directories. |
| Turn the limits off again with `SET`. | Lock the configuration after startup. |
| Download an extension at run time. | Disable auto-install and auto-load. |
| Block the async runtime with a slow query. | Run each DuckDB call on a blocking thread. |
| Run a query without end. | Use a deadline. At the deadline, interrupt the query. |
| Lose an interrupt that comes before the query starts. | Mark the ticket as cancelled. Repeat the interrupt until the call ends. |
| Leave a query running after a client disconnects. | A drop guard interrupts the query. |
| Use more connections than the limit. | A semaphore permit stays with the connection until the blocking call ends. |
| Give the next caller a connection with an open transaction. `is_autocommit` always gives `true`. | Always send `ROLLBACK` on return. Drop the connection on an unexpected error. |
| Read a very large result into memory. | A row limit and a byte limit. Too much gives an error, not a partial result. |
| Lose data at shutdown. | Run `CHECKPOINT` on a writable file database. |
| Open an in-memory database in read-only mode. | Validation refuses it. |
| Log secrets or personal data. | Do not log SQL text, parameter values or DuckDB messages. A constraint message can hold a key value. |
| Send internal DuckDB messages to HTTP clients. | The error text gives the DuckDB error class only. `detail()` gives the full message. |
| Panic in a request path. | No `unwrap`, `expect` or `panic` in library code. Clippy denies them. |
| Start with a bad configuration and fail later. | Validate in `build`. Open the database in the startup hook. A failure stops the boot. |
| Change a managed setting through `settings`. | Validation refuses managed keys in `settings`. |
| Make tests slow or flaky. | Tests use in-memory DuckDB. No test uses the network. |

## 3. Six thinking hats

### White hat (facts)

- Autumn 0.7 gives `Plugin`, `on_startup`, `on_shutdown`, `health_indicator`, `metrics_source` and `config_section`.
- The `duckdb` crate 1.10505 wraps DuckDB. The `bundled` feature builds DuckDB from source.
- A `Connection` is `Send` but not `Sync`. `try_clone` gives a new connection to the same database.
- `InterruptHandle::interrupt` stops the running query on one connection. The query fails with an `INTERRUPT Error`.
- An interrupt before the query starts has no effect. A probe proved it.
- `Connection::prepare` runs each statement before the last one. It does not refuse a batch.
- `Connection::is_autocommit` always gives `true`.
- DuckDB error messages can hold SQL text and key values.
- `duckdb-rs` cannot bind lists, structs or maps as parameters.
- One process can open a file database for writes. Many processes can open it read-only.
- DuckDB error messages start with the error class, for example `Catalog Error: ...`.

### Red hat (feelings)

- Users want one line of setup and one line for a query.
- A query that reads server files is the worst result. File access must be off by default.
- Silent truncation of results feels unsafe.

### Black hat (risks)

- The bundled build compiles C++. The first build is slow. CI must cache it.
- DuckDB can change error text between releases. The class parser must accept unknown text.
- A timestamp with a time zone and one without give the same `duckdb-rs` value. We give UTC text for both.
- A blocking thread keeps running after a timeout until DuckDB sees the interrupt.

### Yellow hat (benefits)

- In-memory DuckDB is fast and real. Tests need no fake and no network.
- Prepared statements remove the main SQL injection path.
- A locked configuration and no external access limit the damage of an injection.

### Green hat (new ideas)

- A setup hook runs with full access before the plugin applies the limits. It can create views and load data.
- `with_connection` gives the full `duckdb` API under the same timeout and limits.
- A property test compares our date text with DuckDB's own cast.

### Blue hat (process)

- Pure modules first. Each gets a `# Contract` section and tests.
- Glue modules next. Each gets tests on in-memory DuckDB.
- Each cycle is red, then green, then refactor. Each phase gets a commit.
- At the end, review agents check the code from different angles.

## 4. Decisions

### In scope

Ideas 1 to 23.

### Out of scope

| Idea | Reason |
|------|--------|
| 24. Arrow and Polars | Large dependencies. `with_connection` gives the `duckdb` Arrow API. |
| 25. Streaming rows | The row limit covers the main need. Add later if users ask. |
| 26. MotherDuck | It needs the network. Validation refuses `md:` paths. |
| 27. Migrations | Use the setup hook. |
| 28. A test fake | In-memory DuckDB is the test database. |
| 29. More than one database | The plugin name is fixed. Use `ATTACH` in the setup hook. |

## 5. Architecture

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

## 6. TDD plan

Each item is one cycle. Red: write a test that fails. Green: write the minimum code. Refactor: clean up with all tests green.

1. `statement`: counts in plain SQL, literals, identifiers, dollar quotes and comments. Property: the count equals the DuckDB count.
2. `temporal`: known dates, times and timestamps. Property: date text equals the DuckDB cast.
3. `param`: each Rust type converts to the correct `Param`.
4. `value`: each DuckDB value converts. Size counts text, blobs and nested values.
5. `decode`: structs, options, numbers, decimals, lists, maps and enums decode. Wrong types give an error.
6. `error`: the class parser and the HTTP status map.
7. `config`: defaults, TOML, profile layers, environment variables and validation.
8. `pool`: open, access limits, lock, reuse and rollback on return.
9. `client`: fetch, execute, parameter count, limits, timeout, drop and shutdown.
10. `plugin`: the extractor, health, metrics and boot errors, in `TestApp`.

## 7. Review

Review agents read the code after the build. Each agent has one angle. This section records the findings and the fixes.
