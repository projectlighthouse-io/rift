# Decisions

Format: date, decision, reason.

## 2026-11-20 — M0

- **Workspace members listed explicitly.** `crates/*` would also match `crates/drivers`
  (no manifest); `crates/drivers/*` is globbed.
- **Edition 2024, resolver 3, MSRV 1.90.** Matches the local toolchain; GPUI tracks recent
  stable.
- **Package metadata inherited from `[workspace.package]`,** including `publish = false`.
- **`registry` enables every driver feature by default.** Release builds ship all drivers;
  tests and slim builds use `--no-default-features --features ...`.
- **`clippy.toml` allows `unwrap`/`expect` in tests** (`allow-unwrap-in-tests`), so test
  modules need no per-file `allow`.
- **Dependency check uses `cargo tree --prune registry`.** Anything with a `-driver` name left
  in `app`'s tree after pruning `registry` is a violation. The script also checks the direct
  workspace deps of `types`, `traits`, `dialect` and every driver.
- **Dev tasks via `Makefile`** (`make dev-up`, `dev-seed`, `dev-down`); `make` is everywhere.
- **Redis seed runs inside the container** (`redis-cli --pipe`), so no local `redis-cli` is
  needed.
- **Third-party dependency versions are pinned when first used** (M1 onward) and recorded
  here; M0 stubs have none.
- **Host ports overridable** via `RIFT_PG_PORT` / `RIFT_REDIS_PORT` (defaults 5432 / 6379) for
  machines where another Postgres or Redis already holds the default port.

## 2026-11-20 — M1

- **Futures are `futures::future::BoxFuture`,** not `async fn` in traits: capability traits
  must be object safe (`&dyn Browse`).
- **`Query::execute` runs one statement.** Splitting scripts into statements happens in
  `dialect` (via `sqlparser`) so every driver gets the same behaviour.
- **Rows stream as `BoxStream<Result<Vec<Row>>>`** of up to `PAGE_SIZE` (500) rows; drivers
  feed it from a bounded channel of 4 pages.
- **`Transactions::atomic(&[TxOp])`** covers both families: SQL statements or key writes.
  Unsupported ops fail before anything is written.
- **Stores are synchronous.** They are local SQLite; the app calls them off the UI thread.
- **Conformance fixture:** a 3-row table `(id, name, score)` the driver harness creates. SQL
  suites restore it after mutating, so all suites share one table.
- **`registry` feature `fake`** adds `testkit`'s `FakeSqlFactory` and `FakeKvFactory`; CI runs
  `cargo test -p registry --features fake`.

## 2026-11-20 — M2

- **Rust 1.98 pinned** in `rust-toolchain.toml`: `gpui-pre-macos` 0.3.12 uses
  `Option::map_or_default`, stable only from 1.97/1.98.
- **GPUI:** `gpui-component =0.10.0`, which requires `gpui = { package = "gpui-pre",
  version = "=0.3.12" }` and `gpui_platform = { package = "gpui-pre-platform",
  version = "=0.3.12" }` (`Application::with_platform(gpui_platform::current_platform(false))`).
  `runtime_shaders` avoids needing the Metal compiler from full Xcode.
- **sqlx 0.9 for Postgres (and later MySQL) instead of tokio-postgres / mysql_async**
  (owner's call). sqlx stays private to the driver crates; the app only sees the traits.
  - Results use `raw_sql` (simple protocol, text format), so every type is readable; unknown
    types become `Value::Unknown` with the server's text.
  - Result columns come from `Executor::prepare` before running the statement.
  - sqlx has no cancel API: the driver records `pg_backend_pid()` and runs
    `pg_cancel_backend(pid)` from a side connection when the token fires.
  - SQLite keeps `rusqlite` so long statements can be interrupted.
- **Values are inlined as SQL literals** by `dialect` (filters, edits) rather than bound as
  parameters: untyped string literals coerce to enums, numerics and timestamps in every engine,
  and the generated SQL is what the status bar and snapshots show. Escaping is per dialect.
- **Driver integration tests skip when Docker is unreachable,** unless `RIFT_REQUIRE_DOCKER=1`.
  Ports come from `RIFT_PG_PORT` etc.
- **Custom grid on GPUI `uniform_list`** instead of `gpui-component`'s `DataTable`: the design
  needs per-cell edit states, a row-number column, header name + type + key icons and a focus
  ring; owning ~300 lines is simpler than restyling the component. 100k rows scroll with no
  frame over 16 ms in release (`RIFT_DEV_AUTOSCROLL=1 RIFT_FRAME_LOG=1`, see below).
- **Icons are the mockups' own SVGs,** extracted from `design/html` into `crates/app/assets/icons`
  and served under `rift/`. Other paths fall back to `gpui-kit-assets`.
- **Source Serif 4 is bundled as static TTFs** (Regular, Italic, Semibold, Bold, OFL). Adobe ships
  no static Medium (500), so 500 renders with the nearest weight.
- **Mono font:** first installed of SF Mono, Menlo, JetBrains Mono, Consolas, DejaVu Sans Mono.
- **Window keeps the native title bar;** the 1440×900 design is the content area below it.
- **Dev switches:** `RIFT_DEV_QUERY` opens a result tab with that SQL at startup;
  `RIFT_DEV_AUTOSCROLL=1` scrolls every row one step per frame; `RIFT_FRAME_LOG=1` logs slow
  frames in release builds (debug builds always do).
