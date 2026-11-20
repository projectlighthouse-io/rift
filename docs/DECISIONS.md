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
