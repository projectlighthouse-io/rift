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
